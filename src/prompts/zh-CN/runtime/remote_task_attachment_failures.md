本工作项的部分需求附件未能下载到本地（清单如下）。执行中如需使用这些文件，先向用户说明缺失情况并确认替代来源，不要臆测文件内容：
{% for item in failures %}- {{ item.filename }}（{{ item.reason }}）
{% endfor %}