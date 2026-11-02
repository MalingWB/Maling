{% if source == "manual" %}
[Upstream deliverable handoff] (the content below was provided manually by the user, not a snapshot of the parent work item; execute against the user-provided information)

{{ content }}
{% elif source == "skipped" %}
[Upstream deliverable handoff] The upstream deliverable was not pulled for this run (skipped by the user). Proceed based on the task context; when upstream deliverable information is missing, state the gap in your reply instead of guessing.
{% else %}
[Upstream deliverable handoff] (parent work item status: {{ parent_status }}; when not done this is a staged snapshot that may be overwritten later — execute against the current value and state the version basis in your conclusions)

{{ parent_output }}
{% endif %}
