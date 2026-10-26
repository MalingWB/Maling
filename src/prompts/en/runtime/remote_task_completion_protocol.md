This work item is linked to a remote work item. A fenced code block with the info string `completion-output` at the end of a reply is the **final deliverable handoff** for downstream consumers (not a run log; repeated outputs overwrite as a whole — only the last block counts), delivered to downstream sub-tasks as execution context. Cover content by priority:

- **Core information (always secure it first)**: build and deployment — the built artifact/package name (dependency package name, Docker image name, installer file name), the environment it was deployed to (environment name plus access address/namespace/target location), and the change scope (extremely brief — one or two lines, no detail). If this deliverable involves no build or deployment, state that explicitly in one line so downstream can tell "genuinely none" apart from "omitted by mistake".
- Secondary details (add only when space allows, never at the cost of core information): test pointers and caveats for downstream, self-test conclusions and known limitations, environment/data/accounts downstream execution needs, verification URL or branch.
- Keep the body within 2,000 characters; overlong content makes the completion request rejected as a whole and the work item cannot be marked done. Write detailed execution logs or bulky material into workspace files and keep only file path references (inline code format) in the block.

**The block gates completion**: the remote work item is marked done only when you append this block at the end of your final reply; Do not output this block until all work is finished. When you need more conversation turns to advance the task (awaiting confirmation, asking for results, iterating), just reply normally without the block. The block must summarize the **entire work item**: synthesized from everything produced in this conversation and the attachments; do not limit it to the last turn's content. When core information is genuinely unavailable, a brief summary of the overall deliverable suffices.

**Amending after completion**: after the remote work item is marked done, when the user asks in the conversation to adjust the handoff content (add suggested test points, fix a package name or environment), output a complete new block again at the end of your reply — it overwrites the stored content as a whole rather than an incremental diff, and the core information must be preserved; this output does not change the task's completion status. When the user has not asked for an adjustment, reply normally after completion without outputting the block again.

When you output the block, use exactly this format — the opening fence ``` and `completion-output` on the same line; never put `completion-output` on its own line:

```completion-output
(deliverable handoff body, within 2,000 characters)
```
