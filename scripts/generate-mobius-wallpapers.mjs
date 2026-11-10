// Generates the Möbius builtin-theme wallpapers (dark/light) from the Gold Band logo palette.
// Renders the logo's infinity band as one continuous crisp ribbon: constant-width quad
// strips with analytic anti-aliasing (3x vertical subsampling), a continuous color
// cross-fade at the extremes (a true Möbius band is a single surface), a clean weave gap
// at the central crossing, and tapered amber crescents inside the loop openings.
// Zero-dependency. Usage: node scripts/generate-mobius-wallpapers.mjs
import { deflateSync } from 'node:zlib';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(repoRoot, 'themes', 'mobius', 'assets', 'wallpapers');

const WIDTH = 2048;
const HEIGHT = 1280;
const SUB_Y = [0.125, 0.375, 0.625, 0.875]; // vertical subsampling rows per pixel row

const BRAND = {
  vermilion: [0xcc, 0x33, 0x2b],
  cobalt: [0x24, 0x48, 0x92],
  amber: [0xe9, 0x9d, 0x54],
};

// ---------------------------------------------------------------------------
// Curve: the two crossing sweeps of the lemniscate of Bernoulli.
function sampleSweep({ centerX, centerY, span, tilt, thetaFrom, thetaTo, samples }) {
  const points = [];
  const cos = Math.cos(tilt);
  const sin = Math.sin(tilt);
  for (let i = 0; i <= samples; i += 1) {
    const theta = thetaFrom + ((thetaTo - thetaFrom) * i) / samples;
    const d = 1 + Math.sin(theta) ** 2;
    const x = (span * Math.cos(theta)) / d;
    const y = (span * Math.sin(theta) * Math.cos(theta)) / d;
    points.push([centerX + x * cos - y * sin, centerY + x * sin + y * cos]);
  }
  return points;
}

function segmentFrames(points) {
  const frames = [];
  for (let i = 0; i + 1 < points.length; i += 1) {
    const [x0, y0] = points[i];
    const [x1, y1] = points[i + 1];
    const dx = x1 - x0;
    const dy = y1 - y0;
    const length = Math.hypot(dx, dy) || 1;
    frames.push({ p0: points[i], p1: points[i + 1], nx: -dy / length, ny: dx / length, t: i / (points.length - 1) });
  }
  return frames;
}

// ---------------------------------------------------------------------------
// Layer: per-pixel best coverage with its color, max-combined across quads.
function newLayer() {
  return { a: new Float32Array(WIDTH * HEIGHT), rgb: new Array(WIDTH * HEIGHT) };
}

function eraseQuad(layer, corners, widthPad) {
  // Clears layer coverage under a widened footprint — used to cut the weave gap.
  drawQuad(layer, corners, widthPad, { rgba: null, alpha: 1, shade: 0, erase: true });
}

function drawQuad(layer, corners, widthPad, { rgba, alpha, shade, erase = false }) {
  const padded = padCorners(corners, widthPad);
  const ys = padded.map(([, y]) => y);
  const yStart = Math.max(0, Math.floor(Math.min(...ys) - 0.5));
  const yEnd = Math.min(HEIGHT - 1, Math.ceil(Math.max(...ys) + 0.5));
  for (let y = yStart; y <= yEnd; y += 1) {
    // Average horizontal coverage over the sub-sampled scanlines, then paint once.
    const coverage = new Map();
    let spans = [];
    for (const subY of SUB_Y) {
      const yc = y + subY;
      const xs = [];
      for (let i = 0; i < 4; i += 1) {
        const [ax, ay] = padded[i];
        const [bx, by] = padded[(i + 1) % 4];
        if ((ay <= yc && by > yc) || (by <= yc && ay > yc)) {
          xs.push(ax + ((yc - ay) / (by - ay)) * (bx - ax));
        }
      }
      if (xs.length < 2) continue;
      xs.sort((a, b) => a - b);
      for (let pair = 0; pair + 1 < xs.length; pair += 2) spans.push([xs[pair], xs[pair + 1]]);
    }
    if (!spans.length) continue;
    for (const [xFrom, xTo] of spans) {
      const xStart = Math.max(0, Math.floor(xFrom));
      const xEnd = Math.min(WIDTH - 1, Math.ceil(xTo));
      for (let x = xStart; x <= xEnd; x += 1) {
        const overlap = Math.min(x + 1, xTo) - Math.max(x, xFrom);
        if (overlap <= 0) continue;
        coverage.set(x, (coverage.get(x) ?? 0) + overlap / SUB_Y.length);
      }
    }
    for (const [x, value] of coverage) {
      const index = y * WIDTH + x;
      if (erase) {
        layer.a[index] = 0;
        layer.rgb[index] = null;
        continue;
      }
      const across = 0; // shading sampled at span center below
      void across;
      const shaded = alpha * Math.min(1, value) * shadeFactor(shade, x, spans);
      if (shaded > layer.a[index]) {
        layer.a[index] = shaded;
        layer.rgb[index] = rgba;
      }
    }
  }
}

function padCorners(corners, pad) {
  if (!pad) return corners;
  const cx = corners.reduce((sum, [x]) => sum + x, 0) / 4;
  const cy = corners.reduce((sum, [, y]) => sum + y, 0) / 4;
  return corners.map(([x, y]) => {
    const dx = x - cx;
    const dy = y - cy;
    const length = Math.hypot(dx, dy) || 1;
    return [x + (dx / length) * pad, y + (dy / length) * pad];
  });
}

// Slight darkening toward the span ends keeps flat bands from reading as tubes.
function shadeFactor(shade, x, spans) {
  if (!shade) return 1;
  let nearest = spans[0];
  for (const span of spans) if (Math.abs((span[0] + span[1]) / 2 - x) < Math.abs((nearest[0] + nearest[1]) / 2 - x)) nearest = span;
  const center = (nearest[0] + nearest[1]) / 2;
  const half = Math.max(1e-6, (nearest[1] - nearest[0]) / 2);
  const across = Math.max(-1, Math.min(1, (x - center) / half));
  return 1 - shade * across * across;
}

// One continuous ribbon: constant-width quad strip, per-segment color so the
// strand can cross-fade into the next color at the extremes (a closed Möbius band).
function ribbonLayer(frames, halfWidth, { colorAt, alpha, shade }) {
  const layer = newLayer();
  for (const frame of frames) {
    const rgba = colorAt(frame.t);
    if (!rgba) continue;
    drawQuad(layer, quadCorners(frame, -halfWidth, halfWidth), 0, { rgba, alpha, shade });
  }
  return layer;
}

function quadCorners({ p0, p1, nx, ny }, r0, r1) {
  return [
    [p0[0] + nx * r0, p0[1] + ny * r0],
    [p1[0] + nx * r0, p1[1] + ny * r0],
    [p1[0] + nx * r1, p1[1] + ny * r1],
    [p0[0] + nx * r1, p0[1] + ny * r1],
  ];
}

// Loop interior mask (even-odd scanline fill of the closed loop polygon) used to
// keep the amber crescents inside the loop openings like the logo.
function interiorSpans(points) {
  const rows = Array.from({ length: HEIGHT }, () => []);
  for (let i = 0; i + 1 < points.length; i += 1) {
    const [ax, ay] = points[i];
    const [bx, by] = points[i + 1];
    const yStart = Math.max(0, Math.ceil(Math.min(ay, by) - 0.5));
    const yEnd = Math.min(HEIGHT - 1, Math.floor(Math.max(ay, by) + 0.5));
    for (let y = yStart; y <= yEnd; y += 1) {
      const yc = y + 0.5;
      if ((ay <= yc && by > yc) || (by <= yc && ay > yc)) {
        rows[y].push(ax + ((yc - ay) / (by - ay)) * (bx - ax));
      }
    }
  }
  return rows.map((crossings) => crossings.sort((a, b) => a - b));
}

function spanContains(crossings, x) {
  let inside = false;
  for (const crossing of crossings) {
    if (crossing > x) break;
    inside = !inside;
  }
  return inside;
}

// Tapered crescent: hugs the inner edge of the loop band only across an arc
// window, fading out at both ends — the logo's moon-crescent accent, not a ring.
// `side` picks which normal direction faces the loop opening (the two loops are
// traversed in opposite orientations, so the inner side flips).
function crescentLayer(loopFrames, loopPolygon, { side, offset, width, rgba, alpha, window: [from, to], fade }) {
  const spans = interiorSpans(loopPolygon);
  const layer = newLayer();
  for (const frame of loopFrames) {
    const strength = windowStrength(frame.t, from, to, fade);
    if (strength <= 0) continue;
    drawQuad(layer, quadCorners(frame, -side * offset, -side * (offset + width)), 0, { rgba, alpha: alpha * strength, shade: 0.1 });
  }
  for (let y = 0; y < HEIGHT; y += 1) {
    for (let x = 0; x < WIDTH; x += 1) {
      const index = y * WIDTH + x;
      if (layer.a[index] > 0 && !spanContains(spans[y] ?? [], x + 0.5)) layer.a[index] = 0;
    }
  }
  return layer;
}

function windowStrength(t, from, to, fade) {
  if (t <= from - fade || t >= to + fade) return 0;
  if (t < from) return smoothstep((t - (from - fade)) / fade);
  if (t > to) return smoothstep((to + fade - t) / fade);
  return 1;
}

function smoothstep(value) {
  const clamped = Math.max(0, Math.min(1, value));
  return clamped * clamped * (3 - 2 * clamped);
}

// ---------------------------------------------------------------------------
function compose(layers) {
  const rgba = Buffer.alloc(WIDTH * HEIGHT * 4);
  for (let i = 0; i < WIDTH * HEIGHT; i += 1) {
    let bestA = 0;
    let bestRgb = null;
    for (const layer of layers) {
      if (layer.a[i] > bestA) {
        bestA = layer.a[i];
        bestRgb = layer.rgb[i];
      }
    }
    if (!bestRgb || bestA <= 0.002) continue;
    const index = i * 4;
    rgba[index] = bestRgb[0];
    rgba[index + 1] = bestRgb[1];
    rgba[index + 2] = bestRgb[2];
    rgba[index + 3] = Math.min(255, Math.round(Math.min(1, bestA) * 255));
  }
  return rgba;
}

function mixColor(a, b, ratio) {
  return [
    Math.round(a[0] + (b[0] - a[0]) * ratio),
    Math.round(a[1] + (b[1] - a[1]) * ratio),
    Math.round(a[2] + (b[2] - a[2]) * ratio),
  ];
}

// Continuous-band color: vermilion across the strand, cross-fading into cobalt
// over the final FADE fraction of each end so the two sweeps close into one band.
function strandColor(base, other, t, fade) {
  if (t < fade) return mixColor(other, base, smoothstep(t / fade));
  if (t > 1 - fade) return mixColor(base, other, smoothstep((t - (1 - fade)) / fade));
  return base;
}

function renderWallpaper({ bandHalfWidth, vermilion, cobalt, amber, bandAlpha, crescentAlpha, shadowAlpha }) {
  const geometry = { centerX: WIDTH * 0.5, centerY: HEIGHT * 0.52, span: WIDTH * 0.46, tilt: -0.32 };
  const fade = 0.16;
  // Sweep A (θ 0→π): rightmost → upper-right → center → lower-left → leftmost.
  // Sweep B (θ π→2π): its mirror. Each fades into the other at both extremes.
  const sweepA = segmentFrames(sampleSweep({ ...geometry, thetaFrom: 0, thetaTo: Math.PI, samples: 1200 }));
  const sweepB = segmentFrames(sampleSweep({ ...geometry, thetaFrom: Math.PI, thetaTo: 2 * Math.PI, samples: 1200 }));
  const rightLoop = sampleSweep({ ...geometry, thetaFrom: -Math.PI / 2, thetaTo: Math.PI / 2, samples: 720 });
  const leftLoop = sampleSweep({ ...geometry, thetaFrom: Math.PI / 2, thetaTo: (3 * Math.PI) / 2, samples: 720 });

  const crescentRight = crescentLayer(segmentFrames(rightLoop), rightLoop, {
    side: -1, offset: bandHalfWidth + 5, width: 30, rgba: amber, alpha: crescentAlpha, window: [0.28, 0.7], fade: 0.14,
  });
  const crescentLeft = crescentLayer(segmentFrames(leftLoop), leftLoop, {
    side: 1, offset: bandHalfWidth + 5, width: 30, rgba: amber, alpha: crescentAlpha, window: [0.3, 0.72], fade: 0.14,
  });

  // Draw sweep B, cut a weave gap under sweep A's footprint, then lay a soft dark
  // shadow around the over-strand so the crossing reads with depth, and draw sweep A.
  const bandB = ribbonLayer(sweepB, bandHalfWidth, {
    colorAt: (t) => strandColor(cobalt, vermilion, t, fade),
    alpha: bandAlpha,
    shade: 0.16,
  });
  const crossingFrames = sweepA.filter((frame) => frame.t > 0.38 && frame.t < 0.62);
  for (const frame of crossingFrames) eraseQuad(bandB, quadCorners(frame, -bandHalfWidth, bandHalfWidth), 6);
  const shadow = ribbonLayer(crossingFrames, bandHalfWidth + 5, {
    colorAt: () => [0, 0, 0],
    alpha: shadowAlpha,
    shade: 0,
  });
  const bandA = ribbonLayer(sweepA, bandHalfWidth, {
    colorAt: (t) => strandColor(vermilion, cobalt, t, fade),
    alpha: bandAlpha,
    shade: 0.16,
  });

  return compose([crescentRight, crescentLeft, bandB, shadow, bandA]);
}

// ---------------------------------------------------------------------------
// PNG encoding (RGBA8, zlib).
function crc32(buffer) {
  let c;
  const table = crc32.table ?? (crc32.table = (() => {
    const t = new Int32Array(256);
    for (let n = 0; n < 256; n += 1) {
      c = n;
      for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      t[n] = c;
    }
    return t;
  })());
  c = 0xffffffff;
  for (const byte of buffer) c = table[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([length, body, crc]);
}

function encodePng(rgba) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(WIDTH, 0);
  ihdr.writeUInt32BE(HEIGHT, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // color type RGBA
  const scanlines = Buffer.alloc((WIDTH * 4 + 1) * HEIGHT);
  for (let y = 0; y < HEIGHT; y += 1) {
    scanlines[y * (WIDTH * 4 + 1)] = 0; // filter: none
    rgba.copy(scanlines, y * (WIDTH * 4 + 1) + 1, y * WIDTH * 4, (y + 1) * WIDTH * 4);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(scanlines, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

function lift([r, g, b], factor) {
  return [
    Math.min(255, Math.round(r + (255 - r) * factor)),
    Math.min(255, Math.round(g + (255 - g) * factor)),
    Math.min(255, Math.round(b + (255 - b) * factor)),
  ];
}

mkdirSync(outDir, { recursive: true });
const wallpapers = [
  {
    file: 'mobius-band-dark.png',
    params: {
      bandHalfWidth: 34,
      bandAlpha: 0.4,
      crescentAlpha: 0.3,
      shadowAlpha: 0.42,
      // Lift the brand trio slightly so the bands read on the near-black canvas.
      vermilion: lift(BRAND.vermilion, 0.13),
      cobalt: lift(BRAND.cobalt, 0.28),
      amber: lift(BRAND.amber, 0.12),
    },
  },
  {
    file: 'mobius-band-light.png',
    params: {
      bandHalfWidth: 34,
      bandAlpha: 0.18,
      crescentAlpha: 0.17,
      shadowAlpha: 0.1,
      vermilion: BRAND.vermilion,
      cobalt: BRAND.cobalt,
      amber: BRAND.amber,
    },
  },
];
for (const { file, params } of wallpapers) {
  const png = encodePng(renderWallpaper(params));
  writeFileSync(join(outDir, file), png);
  console.log(`${file}: ${WIDTH}x${HEIGHT}, ${(png.length / 1024).toFixed(0)} KiB`);
}
