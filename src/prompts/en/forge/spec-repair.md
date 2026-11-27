Saving failed; repairing {{ attempt }}/{{ max_attempts }}

<hidden data-gold-band-hidden="true" title="Forge Spec repair">
The Forge Spec delta you output last round failed the pre-save validation after merging. Fix it and output the delta again; do not alter conclusions the user has already confirmed.

Structured errors (error code → detail, all listed at once):

{{ errors }}

Repair requirements:

1. Fix only the stages and fields the errors point at; leave everything else unchanged. Locate the failing stage in the "Current baseline Spec" and carry its `id` when modifying it;
2. Output the repaired delta in the same incremental JSON structure as before (a single code fence, still a delta — not the full Spec); fix every fixable problem in this one round instead of one per round;
3. Fix structural errors directly:
   - `forge/unknown-failure-target`: the failure target points at a stage that does not exist. Change it to `"retry"` and add "where should this stage go when it fails?" to `openQuestions` with your recommendation;
   - `forge/role-id-conflict`, `forge/role-definition-conflict`: Use one consistent charter for all stages sharing a role, based on the current baseline. Omit `profileId`; only the system binds a role.
   - `forge/readonly-role`: Library references without an editable charter are read-only: keep the name and omit `roleDefinition`.
4. Never fabricate an answer to something the user has to decide: put it into `openQuestions` with your recommendation and say so in your reply. That covers:
   - `forge/missing-goal-element`, `forge/unresolved-questions`: a stage goal lacks an element, or an open question has not been settled yet;
   - `forge/duplicate-workflow-name`: the workflow name is already taken. Recommend a name that tells them apart and ask the user to confirm; do not rename it on your own.
</hidden>
