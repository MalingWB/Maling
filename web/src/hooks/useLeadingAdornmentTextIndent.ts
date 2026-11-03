import { useLayoutEffect, useRef, useState, type CSSProperties } from 'react';

/// leading adornment 机制的统一间距：既用于正文首行缩进的让位余量（槽宽 + gap），
/// 也用于槽内并排标签（slash 标签 + 远程绑定 chip）的相互间距——单一来源，改节奏两处同步。
export const LEADING_ADORNMENT_GAP = '0.25rem';

export function useLeadingAdornmentTextIndent(enabled: boolean) {
  const adornmentRef = useRef<HTMLSpanElement>(null);
  const [adornmentWidth, setAdornmentWidth] = useState(0);

  useLayoutEffect(() => {
    if (!enabled || !adornmentRef.current) return undefined;
    const adornment = adornmentRef.current;
    const updateWidth = () => setAdornmentWidth(adornment.getBoundingClientRect().width);
    updateWidth();
    const observer = new ResizeObserver(updateWidth);
    observer.observe(adornment);
    return () => observer.disconnect();
  }, [enabled]);

  const textareaStyle: CSSProperties | undefined = enabled && adornmentWidth > 0
    ? { textIndent: `calc(${adornmentWidth}px + ${LEADING_ADORNMENT_GAP})` }
    : undefined;

  return { adornmentRef, textareaStyle };
}
