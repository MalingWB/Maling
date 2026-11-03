import type { Ref } from 'react';
import { useTranslation } from 'react-i18next';
import { Globe, X } from 'lucide-react';
import { Badge } from '@/components/ui/badge';
import { SlashCommandInputTag } from '@/components/conversation/SlashCommandInputTag';
import { COMPOSER_LEADING_ADORNMENT_SLOT_CLASS_NAME } from '@/lib/conversation-composer-layout';
import { LEADING_ADORNMENT_GAP } from '@/hooks/useLeadingAdornmentTextIndent';
import type { ConversationComposerRemoteBinding } from '@/lib/conversation-composer-draft';
import { remoteTaskSourceLabel } from '@/lib/remote-sources';
import { agentIconClass, agentIconSrc } from '@/lib/agent-icons';
import type { CommittedSlashItem } from '@/lib/slash-command';

/**
 * composer 正文首行的 leading adornment 槽：已提交 slash/role 标签与远程任务绑定 chip 并排呈现。
 *
 * 二者是合法组合——用户点需求管理「执行」预填远程任务正文后，在开头追加 `@角色 ` 指定执行
 * 角色，发送链路同时转发 role snapshot 与 draft.remote（start_remote_conversation_run）。
 * 槽容器挂 useLeadingAdornmentTextIndent 的 adornmentRef：首行按槽总宽缩进让位，换行回到
 * 左缘（标准 CSS text-indent 只作用于首行）。slash 标签锚定正文第一个 token，排在最前。
 */
interface ComposerLeadingAdornmentProps {
  /// 已提交的 slash/role 项（正文开头的 `@role `/`/cmd ` token），null = 未提交。
  committedSlashCommand: CommittedSlashItem | null;
  /// 远程任务绑定（需求管理「点击执行」prefill 的 draft.remote），null = 未绑定。
  remoteBinding: ConversationComposerRemoteBinding | null;
  /// slash 标签的 agent 图标 key（kind=command 时按 agent 取图标）。
  slashAgentIconKey?: string;
  /// 槽容器 ref：由 useLeadingAdornmentTextIndent 提供，用于测量总宽做首行缩进。
  adornmentRef: Ref<HTMLSpanElement>;
  /// 解除远程绑定（点 chip ×）。
  onUnbindRemoteTask: () => void;
}

export function ComposerLeadingAdornment({
  committedSlashCommand,
  remoteBinding,
  slashAgentIconKey,
  adornmentRef,
  onUnbindRemoteTask,
}: ComposerLeadingAdornmentProps) {
  const { t } = useTranslation();
  if (!committedSlashCommand && !remoteBinding) return null;
  return (
    // max-w-full：窄窗口下槽宽受滚动容器约束（首行 text-indent = 槽总宽 + gap 不超出行宽），
    // 溢出压力由 chip 内 truncate 消化（min-w-0 允许 Badge 随槽收缩）。
    <span
      ref={adornmentRef}
      className={`${COMPOSER_LEADING_ADORNMENT_SLOT_CLASS_NAME} left-0 top-2 max-w-full`}
      style={{ gap: LEADING_ADORNMENT_GAP }}
    >
      {committedSlashCommand ? (
        <SlashCommandInputTag
          prefix={committedSlashCommand.prefix}
          description={committedSlashCommand.item.description}
          content={committedSlashCommand.item.content}
          kind={committedSlashCommand.item.kind}
          iconSrc={committedSlashCommand.item.kind === 'role'
            ? '/logo.svg'
            : slashAgentIconKey
              ? agentIconSrc(slashAgentIconKey)
              : '/logo.svg'}
          iconClassName={committedSlashCommand.item.kind === 'command' && slashAgentIconKey
            ? agentIconClass(slashAgentIconKey)
            : undefined}
        />
      ) : null}
      {remoteBinding ? (
        /* accent/accent-foreground is the theme contract's guaranteed-contrast pair for
           emphasized surfaces (same pairing as permission-card and recipe hover/selected
           states). Never tint this chip from `primary` alone: in themes like
           tech-neutral dark, primary (#2d2d2d) sits nearly on the composer background
           (#1b1b1b) and the chip becomes unreadable. */
        <Badge
          variant="secondary"
          className="min-w-0 gap-1 h-6 rounded-md border-accent-foreground/15 bg-accent px-2 text-[0.75rem] font-medium text-accent-foreground"
        >
          <Globe className="size-3 shrink-0" />
          <span className="max-w-[260px] truncate">
            {t('conversation.composer.remoteBindingTag', {
              source: remoteTaskSourceLabel(t, remoteBinding.source),
              title: remoteBinding.title,
            })}
          </span>
          <button
            type="button"
            aria-label={t('conversation.composer.removeRemoteBinding')}
            onClick={onUnbindRemoteTask}
            className="ml-0.5 inline-flex size-3.5 shrink-0 items-center justify-center rounded-sm hover:bg-accent-foreground/15"
          >
            <X className="size-3" />
          </button>
        </Badge>
      ) : null}
    </span>
  );
}
