This work item is linked to a remote work item. When the remote work item completes, a fenced code block with the info string `completion-output` at the end of the final reply serves as the **final deliverable handoff** for downstream consumers (repeated outputs overwrite the value; keep only the final conclusions — not a run log). Ideally covering:

- Deployment/verification URL or branch, scope of this change, test pointers and caveats for downstream, self-test conclusions and known limitations, environment/data/accounts downstream execution needs
- Keep it within 2,000 characters; overlong content makes the completion request rejected as a whole and the work item cannot be marked done. Write detailed execution logs or bulky material into workspace files and keep only file path references (inline code format) in the block
- The handoff is delivered to downstream sub-tasks as execution context when the work item completes

**When executing as a direct conversation, the block gates completion**: **the remote work item is marked done only when you append this block at the end of your final reply**; until then the task stays in progress and the conversation can continue to drive the work. **Do not output this block until all work is finished**; when you need more conversation turns to advance the task (awaiting confirmation, asking for results, iterating), just reply normally without the block.

**When executing inside a workflow** (you can see a predecessor chain or workflow context): the block does not gate workflow completion, and only the finishing node's block is passed downstream. When you judge that you are the finishing node (your output is the task's final deliverable and will not be further processed by later nodes), output this block at the end of your final reply; if your output will still be processed by later nodes, do not output it. The block must summarize the **entire work item** — synthesize predecessor nodes' outputs and attachments, prioritizing build artifacts, deployment/verification URLs or branches, change scope, and downstream test pointers; when those are not available from predecessor outputs, a brief summary of the overall deliverable is fine; **do not limit the block to your own node's output**.

When you output the block, use exactly this format — the opening fence ``` and `completion-output` on the same line; never put `completion-output` on its own line:

```completion-output
(final deliverable handoff, within 2,000 characters)
```
