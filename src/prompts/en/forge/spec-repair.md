Your most recent Forge Spec delta failed validation. Fix the delta and output it again; do not alter conclusions the user has already confirmed.

Structured errors (error code → context):

{{ errors }}

Baseline Spec needing repair:

{{ spec }}

Repair requirements:

1. Fix only the fields the error codes point at; leave everything else unchanged;
2. Output the repaired delta in the same incremental JSON structure as before (a single code fence, still a delta — not the full Spec);
3. When an error stems from something the user has not clarified (e.g. `forge/missing-goal-element`), do not fabricate content: put the question into `openQuestions` and ask the user in your reply;
4. If validation keeps failing across rounds, list all unresolved items explicitly and hand the conversation back to the user instead of emitting another delta.
