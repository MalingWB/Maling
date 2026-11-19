## New and updated features

> [!IMPORTANT]
> This release migrates your local settings. Version 0.21.x cannot read the migrated settings. If you may need to downgrade, back up your app data before updating.

### 1. Retry the current node or rerun from a failed node in the workflow graph

Open the conversation's workflow run graph and right-click an eligible node:

1. **Retry current node**: If the current node paused unexpectedly before the workflow moved on, run it again within the same Run. The previous conversation and its outputs are kept.
2. **Rerun from this node**: Select a completed attempt that failed, including an eligible past attempt. A new Run starts from that node using the task's latest saved workflow settings. The original Run remains available as read-only history.

> [!WARNING]
> Rerunning from a failed node stops any execution still in progress in the original Run. It does not undo workspace changes, Git commits, or external actions that already occurred. Both options are limited to standard workflow nodes, not AUTO mode or internal AI-DYNAMIC nodes.

### 2. Continue or recover workflows from Feishu and WeCom

Once you've linked a Feishu or WeCom direct chat, the app sends an action card when a workflow pauses in an eligible state. This includes eligible workflows stopped manually in the desktop app:

1. **Continue workflow**: Resume the current node if it hasn't finished.
2. **Recover workflow**: If the current node succeeded but the workflow hasn't moved on, use the existing result to proceed without running the node again.

Notifications are on by default. Turn off **Workflow continuation request** for either channel under **Settings → General → IM remote intervention and notifications**. If a card says the app is still busy just after you stop the workflow, wait a moment and try again on the same card.

> [!NOTE]
> The desktop app must be online. You can't start, stop, or rerun workflows from IM with this update. The app does not send cards in bulk for workflows already paused before the upgrade.

### 3. Three new Agents and more flexible first-run setup

- **Agents** now includes DeepSeek Harness, ZCode, and Qoder. ZCode uses a community adapter. The built-in Qoder option is not available on Windows ARM64. The built-in CodeBuddy entry is now named WorkBuddy by default; existing configurations are kept.
- When adding or editing an Agent, check **Connection requirements** to see which local commands and authentication methods it needs.
- First-run setup still checks Node.js, npm, and npx, but failed checks no longer block detection of local Agents. The results distinguish Agents that aren't installed, those missing runtime requirements, and those that failed diagnostics. Once detection finishes or fails, choose **Set up later** to open the main app and finish setup another time, even if no Agents are available.

### 4. New Vista and Meadow themes, with separate light and dark wallpapers

Choose either new theme under **Settings → Personalization**:

- **Vista**: A watercolor scroll by day, black-and-gold snowy peaks and a starry sky by night. The ink-teal and apricot-gold palettes are drawn from the two wallpapers.
- **Meadow**: The light scheme uses a soft spring green instead of pure white, while the dark scheme evokes a quiet spring night with pine green. Code and Diff areas use brighter backgrounds for clarity.

Gold Band, Vista, and Meadow include six built-in wallpapers in total. Choose **Theme wallpaper**, **Custom**, or **Disable** separately for light and dark modes. Custom wallpapers support **Fill** and **Fit** and can be removed from **Recently used**.

### 5. Jump between conversation turns and resize the input box

- Use the new turn navigation on the left of a conversation. Hover over a marker to preview the message, or click to jump to that turn, including earlier turns not yet on screen.
- On the home and conversation pages, drag the top edge of the input box to adjust its height. The height carries over when you switch conversations. Drag down to a low position to return to automatic sizing.

### 6. Uninstall local roles and workflows from Multica

After connecting, use the **Runtime** tab in Multica's role management area to see the local roles and workflows in the current runtime and uninstall non-built-in items. Uninstalling removes only the local copy, not the content in Multica's shared catalog. When you submit several asset operations in a row, the desktop app processes the queued requests faster.

## Experience improvements

### 1. Easier sidebar and conversation navigation

- Expand run history, rename, or pin directly from **Recent**. Use the buttons at the bottom of sidebar lists to load more. Refreshing run history keeps the current selection in place, and the activity spinner clears when the task ends.
- Markdown table bodies now use the theme's standard text color, making them easier to read in both light and dark themes.

### 2. More reliable startup and Agent connections

- An invalid launch configuration for one Agent no longer prevents the whole app from starting. Fixed an issue where WorkBuddy could switch launch methods on later turns and drop a working local connection.
- Fixed some WebView startup failures. Unsupported environments now show update guidance for the relevant platform. Newer MCP clients can discover the built-in shared memory tools more reliably.

### 3. Knowledge base and download menu on the website

The website and Demo now use the MALING brand. Read the knowledge base directly on the website or use the download menu to select an installer for your platform.
