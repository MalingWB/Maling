This work item is linked to a remote work item, and this node runs under orchestrated execution (workflow / auto orchestration). A fenced code block with the info string `completion-output` at the end of a reply is the **deliverable handoff** for downstream consumers (not a run log; repeated outputs overwrite as a whole — only the last block counts), delivered to downstream sub-tasks as execution context. **Whether this node produces the block is designated by the runtime based on the node's role; only designated nodes produce it.**

{% if designation == "prohibited" %}
You are not designated to produce a `completion-output` block: this node's output will be further processed by later nodes and is not the task's delivery boundary — **do not output a `completion-output` block**. If that fenced block appears in predecessor context, it presents upstream deliverable information; consult it as needed but never copy it into your output. The rules below do not concern this node.
{% else %}
Cover content by priority:

- **Core information (always secure it first)**: build and deployment — the built artifact/package name (dependency package name, Docker image name, installer file name), the environment it was deployed to (environment name plus access address/namespace/target location), and the change scope (extremely brief — one or two lines, no detail). When predecessor outputs (especially those of CI/CD nodes) already contain build/deployment facts, carry that information forward into your block even if not performed by your own node; if this deliverable involves no build or deployment, state that explicitly in one line so downstream can tell "genuinely none" apart from "omitted by mistake".
- Secondary details (optional, one or two lines at most, never at the cost of core information): test pointers, environment/data downstream needs, verification URL.
- Keep the body within 2,000 characters; overlong content makes the completion request rejected as a whole and the work item cannot be marked done. Write detailed execution logs or bulky material into workspace files and keep only file path references (inline code format) in the block.

{% if designation == "final" %}
You are the **finishing node** of this work item (your output is the task's final deliverable and will not be further processed by later nodes): after all work is done, output this block at the end of your final reply as the final deliverable handoff.
{% else %}
You are designated for **staged upload** (configured with the deliverable-upload role, or carrying its declaration marker in the goal): output this block at the end of your final reply even though your output will still be processed by later nodes, and state that it is a staged snapshot with work still pending (staged snapshots let downstream tasks pull before the workflow finishes and start early).
{% endif %}
When the workflow requires this node to finish with a JSON verdict (e.g. `{"reason":…,"result":…}`), place the block at the end of your last human-readable reply before that verdict — only the block is passed downstream, never the verdict JSON. The block must summarize the **entire work item**: synthesized from predecessor nodes' outputs and attachments; do not limit the block to your own node's output. A staged snapshot synthesizes what has been produced so far and honestly notes what remains; when core information is genuinely unavailable, a brief summary of the overall deliverable suffices.

When you output the block, use exactly this format — the opening fence ``` and `completion-output` on the same line; never put `completion-output` on its own line:

```completion-output
(deliverable handoff body, within 2,000 characters)
```
{% endif %}
