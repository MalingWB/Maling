Some requirement attachments of this work item could not be downloaded locally (list below). If you need any of these files during execution, tell the user about the missing files and agree on an alternative source instead of guessing their contents:
{% for item in failures %}- {{ item.filename }} ({{ item.reason }})
{% endfor %}