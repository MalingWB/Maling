## New and Updated Features

### 1. Remote intervention and notifications through Feishu

You can now use Feishu to handle permission requests, Agent follow-up questions, and manual checks while away from your computer, and receive task result notifications.

To get started:

1. Open “Settings → General → IM remote intervention and notifications,” select Feishu, confirm the permissions, and scan the QR code to connect.
2. Send the bot a private message from the account used to authorize the QR code connection to complete the binding.
3. When a card arrives, approve or deny permissions, submit single-choice, multiple-choice, text, and custom answers, or handle manual checks.

After you respond on the desktop, WeCom, or Feishu, the result is synchronized to the other connected channels with the corresponding notifications enabled. Feishu cards retain the answers actually submitted and disable controls that have already been handled.

Only one-to-one private chats with the bound account are currently supported, and each bot connects to one device. Group chats and starting, stopping, or rerunning tasks through IM are not supported. Real-time delivery cannot be guaranteed while the client is offline, the computer is asleep, or the network is disconnected. The international Lark edition has not yet been validated.

> [!WARNING]
> **Review existing cards and downgrade limitations before upgrading**
> This upgrade clears delivery records and associated message bindings for older IM interaction cards. Some cards received before the upgrade may stop working; handle any outstanding requests on the desktop. Older clients cannot directly read the upgraded IM data. Back up your application data before upgrading if you need the option to downgrade.

### 2. Improved native Claude subagent and background task display

When Claude starts a child Agent, you can view its status in the conversation’s Agent row and open read-only details in the right workspace. Synchronous and background subtasks show their own messages, permission requests, and status. Ending the parent turn no longer incorrectly marks a running subtask as interrupted.

Background activity is timed separately, reducing cases where long idle periods are counted as execution time. You can also copy and quote background replies after they finish.

> [!NOTE]
> Claude ACP `0.90.0` or later is required; older versions will prompt you to upgrade. Continuing an existing child Agent after reloading a session remains subject to adapter capabilities. Token totals exclude consumption from background wake-ups, and the panel displays a corresponding warning.

### 3. Improved remote requirement execution and result handoff

“Requirements” in the internal edition improves handling of development tasks, test tasks, and remote attachments:

1. View and execute requirements by development or test task. If the corresponding development task is unfinished, the test task advises you to wait but still allows manual execution.
2. Clicking “Run” brings the requirement text, upstream delivery notes, and other context into the conversation. The task is only formally claimed when you click “Send.”
3. A read-only list of remote attachments is visible before sending. Attachments are downloaded on send and included as inputs for the task. A failed download of an individual attachment does not block the task; the Agent is informed of the missing information.
4. Direct mode waits for explicit completion output before reporting completion, avoiding closing the requirement as soon as the first reply ends. The final workflow step summarizes delivery results for the entire work item for subsequent tasks to use.

### 4. Two-way remote sync for multi-file Skills

Sync complete Skills between your local machine and Multica, including `SKILL.md` and supporting scripts, documentation, and other required files. On the global Skill management page, click “Sync remote SKILLs,” connect to Multica, and select a workspace and remote Skills to pull to your machine and review the sync results. To import local Skills to the remote service, start the operation from Multica.

Fixed stale selections remaining while the remote list reloads and incomplete recovery after sync errors.

> [!CAUTION]
> **Pulling with overwrite replaces the entire Skill directory**
> When you choose to overwrite a local Skill, the remote directory becomes authoritative. Local files that do not exist remotely are also deleted. Save any local changes you need to keep first.

### 5. Open conversation search with a keyboard shortcut

Press `Ctrl+F`, or `Cmd+F` on macOS, to open conversation search and focus the input. Press the shortcut again to select the current query. Press `Esc` to close search and return to the previous control.

The code editor retains priority for its own file search. Conversation search does not open over other dialogs, menus, or selectors.

### 6. Confirm the target before a Git push

Before pushing, Source Control shows the actual “local branch → remote / target branch” so you can check where your code will go. When publishing a branch for the first time, you can select a remote and set the upstream. Branches with an existing upstream push to the target they actually track.

Branches without an upstream are labeled “New branch,” reducing confusion between unpublished and synchronized branches.

## Experience Improvements

### 1. More stable history reading and session recovery

Fixed automatic following pulling you back while scrolling up through history. You can load older history even when the current page does not fill the viewport. Returning to a session no longer lets stale cached data make answered questions appear pending again.

Continuing a session uses its actually saved model and permission settings, reducing failures caused by old workflow configurations.

### 2. Same-name attachments no longer overwrite history

Attachments with the same name but different contents are saved separately. Later uploads no longer overwrite historical attachments or their previews. Long filenames are handled more reliably, with a clear message when the directory path is too deep to store the attachment safely.

### 3. Improved Markdown and file workspace

1. Fixed local image previews in conversations and opening local HTML links, preserving query parameters and in-page locations.
2. Markdown live preview uses the panel’s available width, reducing unnecessary side margins on wide screens.
3. The default editor font size in built-in themes is now `13px`. Existing custom font sizes are preserved.
4. If a right-hand tab encounters a rendering error, you can retry within that tab without affecting the entire conversation. Stability is also improved when multiple file trees coexist.

### 4. Corrected duplicate Token counting

Fixed Token consumption being counted more than once when continuing a conversation after an idle period. Identifiable duplicate totals in existing records are also corrected when read.

### 5. Other improvements

1. Fixed change statistics failing to load on Windows when viewing commits or merge commits with many files.
2. Improved automatic Chinese language matching: explicit Simplified or Traditional script markers take precedence over region. Existing manual language choices are preserved.
3. Completed multilingual copy for sessions kept running and fixed some tooltips not closing promptly.
