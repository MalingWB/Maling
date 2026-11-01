//! multica runtime-assets 工单（角色共享通道 B）处理层。
//!
//! 三种工单（设计 `.claude/design/multica-role-management/` §4；multica 网页「角色管理」页
//! 的上传面板/安装入口遥控本机干活，码灵**纯被动**——唯一主动行为是既有的 15s 心跳认领）：
//! - **list（盘点）**：枚举本地角色，只报摘要不报正文（§5.1，正文仅在 export/install 全量传输）；
//! - **export（提取）**：按 `asset_key`（= profile.id）读完整角色回传，multica 暂存后自行发布；
//! - **push-install（推送安装）**：把 multica 推来的角色正文落为新本地用户角色，回报实际
//!   `installed_name`（= 新 profile.id；与重名/内置冲突时天然落为拷贝，绝不覆盖）。
//!
//! 格式转换（ProfileEntry ↔ 交换文档）的单一真源在 `gold_band::app::profiles`；wire 类型在
//! `client.rs`。本模块只做工单分发与本地读写，与 skill 待办同模型：无状态消费、上报失败经
//! client 内网络重试后记日志放弃、服务端 60s running 超时终态化兜底（对迟到/重复上报幂等）。

use std::collections::BTreeSet;

use gold_band::app::{
    MulticaRoleExchange, ProfileEntry, multica_exchange_to_profile_input,
    profile_to_multica_exchange,
};
use tauri::{AppHandle, Runtime};

use crate::multica::client::{
    MulticaClient, RuntimeAssetSummary, RuntimeAssetsExportPayload, RuntimeAssetsInstallReport,
    RuntimeAssetsInstallResult, RuntimeAssetsListReport, RuntimeAssetsRequest,
};
use crate::multica::local_skills::home_app;

/// list 工单的资产类型（multica §4.3 目前仅 role；其它类型如实回报 `supported: false`）。
const ASSET_TYPE_ROLE: &str = "role";

/// 详情 → 工单分类结果（纯函数，可单测；action 以详情为权威，ack 里的 action 只是建议）。
#[derive(Debug)]
pub(crate) enum RuntimeAssetsWorkOrder {
    /// 盘点：枚举摘要。
    List,
    /// 提取：`item_id` 空 + payload `{asset_key}`；按 key 回读完整角色。
    Export { asset_key: String },
    /// 推送安装：`item_id` 非空 + payload 即角色正文。
    PushInstall { exchange: MulticaRoleExchange },
}

/// 把工单详情分类为三种工单之一；结构非法（payload 缺失/不合规、action 未知）返回错误文案
/// （调用方按 failed 上报终态化，不静默吞掉——网页端要能看到失败原因）。
pub(crate) fn classify_runtime_assets_request(
    detail: &RuntimeAssetsRequest,
) -> Result<RuntimeAssetsWorkOrder, String> {
    match detail.action.as_str() {
        "list" => Ok(RuntimeAssetsWorkOrder::List),
        "install" => {
            let payload = detail
                .payload
                .clone()
                .ok_or_else(|| "runtime-assets request missing payload".to_string())?;
            if detail.item_id.trim().is_empty() {
                let payload: RuntimeAssetsExportPayload = serde_json::from_value(payload)
                    .map_err(|error| format!("export payload invalid: {error}"))?;
                if payload.asset_key.trim().is_empty() {
                    return Err("export payload asset_key empty".to_string());
                }
                Ok(RuntimeAssetsWorkOrder::Export {
                    asset_key: payload.asset_key,
                })
            } else {
                let exchange: MulticaRoleExchange = serde_json::from_value(payload)
                    .map_err(|error| format!("install payload invalid: {error}"))?;
                Ok(RuntimeAssetsWorkOrder::PushInstall { exchange })
            }
        }
        other => Err(format!("unsupported runtime-assets action: {other}")),
    }
}

/// 本地角色列表 → list 工单上报摘要（纯映射；`key` = profile.id，export 工单按它回取）。
///
/// `updated_at` 转 RFC3339（multica 反馈 P4——前端按标准格式解析，本地存储格式是本地约定
/// 不动，上报层适配）。
pub(crate) fn asset_summaries(profiles: &[ProfileEntry]) -> Vec<RuntimeAssetSummary> {
    profiles
        .iter()
        .map(|profile| RuntimeAssetSummary {
            key: profile.id.clone(),
            r#type: ASSET_TYPE_ROLE,
            name: profile.name.clone(),
            description: profile.summary.clone(),
            is_builtin: profile.is_built_in,
            updated_at: rfc3339_from_local_timestamp(&profile.updated_at),
        })
        .collect()
}

/// 本地时间戳（`%Y-%m-%d %H:%M:%S`，本地时区无偏移后缀，`profiles::local_timestamp` 格式）
/// → RFC3339（带本地时区偏移 `±HH:MM`）。
///
/// 解析失败（历史文件格式漂移 / DST 切换歧义时刻）原样返回——展示缺失由前端容错，
/// 好于错误时间。
fn rfc3339_from_local_timestamp(timestamp: &str) -> String {
    chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M:%S")
        .ok()
        .and_then(|naive| naive.and_local_timezone(chrono::Local).single())
        .map(|datetime| datetime.to_rfc3339_opts(chrono::SecondsFormat::Secs, false))
        .unwrap_or_else(|| timestamp.to_string())
}

/// 工单入口：拉详情（权威 action）→ 分类 → 分发三工单处理。
///
/// 详情拉取失败（404 = server 内存态请求已消失/重启、网络失败重试后仍败）直接放弃该工单
/// **不上报**——对不存在的 requestId 上报同样 404，无意义；失败仅记日志。
pub(crate) async fn handle_runtime_assets_request<R: Runtime>(
    app: AppHandle<R>,
    client: MulticaClient,
    runtime_id: String,
    request_id: String,
) {
    let detail = match client
        .get_runtime_assets_request(&runtime_id, &request_id)
        .await
    {
        Ok(detail) => detail,
        Err(error) => {
            tracing::warn!(
                runtime_id = %runtime_id,
                request_id = %request_id,
                %error,
                "multica runtime-assets request detail failed (dropping work order)"
            );
            return;
        }
    };
    let work = classify_runtime_assets_request(&detail);
    match work {
        Ok(RuntimeAssetsWorkOrder::List) => {
            handle_list(app, client, runtime_id, request_id, detail.asset_type).await
        }
        Ok(RuntimeAssetsWorkOrder::Export { asset_key }) => {
            handle_export(app, client, runtime_id, request_id, asset_key).await
        }
        Ok(RuntimeAssetsWorkOrder::PushInstall { exchange }) => {
            handle_push_install(app, client, runtime_id, request_id, exchange).await
        }
        Err(error) => {
            // 结构性失败（payload 非法/action 未知）：failed 上报终态化，网页端可见原因。
            // classify 只对 install 形态工单可能失败，上报走 install 端点。
            report_install(
                &client,
                &runtime_id,
                &request_id,
                failed_install_report(error),
            )
            .await;
        }
    }
}

/// list 工单：枚举本地角色 → 摘要上报（正文不上传）。
///
/// `asset_type` 非 role（multica 后续可能扩展的类型）→ `supported: false` + 空 assets，
/// 如实回报能力（server 据此不再下发该类型工单）。
async fn handle_list<R: Runtime>(
    app: AppHandle<R>,
    client: MulticaClient,
    runtime_id: String,
    request_id: String,
    asset_type: String,
) {
    let report = match home_app(&app) {
        None => failed_list_report("app state unavailable".to_string()),
        Some(app) => {
            // 角色目录扫描是阻塞 FS（十级文件 <10ms），放 spawn_blocking 不占 async worker。
            let scan = tauri::async_runtime::spawn_blocking(move || app.profiles()).await;
            match scan {
                Ok(Ok(list)) => {
                    let supported = asset_type.is_empty() || asset_type == ASSET_TYPE_ROLE;
                    RuntimeAssetsListReport {
                        status: "completed",
                        supported,
                        assets: if supported {
                            asset_summaries(&list.profiles)
                        } else {
                            Vec::new()
                        },
                        error: None,
                    }
                }
                Ok(Err(error)) => failed_list_report(format!("profile scan failed: {error}")),
                Err(error) => failed_list_report(format!("profile scan task failed: {error}")),
            }
        }
    };
    if let Err(error) = client
        .report_runtime_assets_list_result(&runtime_id, &request_id, &report)
        .await
    {
        tracing::warn!(
            runtime_id = %runtime_id,
            request_id = %request_id,
            %error,
            "multica runtime-assets list report failed after retries (server timeout will finalize)"
        );
    }
}

/// export 工单：按 asset_key（profile.id）读完整角色 → 交换文档回传（multica 暂存后自行发布）。
async fn handle_export<R: Runtime>(
    app: AppHandle<R>,
    client: MulticaClient,
    runtime_id: String,
    request_id: String,
    asset_key: String,
) {
    let report = match home_app(&app) {
        None => failed_install_report("app state unavailable".to_string()),
        Some(app) => {
            let key = asset_key.clone();
            let read = tauri::async_runtime::spawn_blocking(move || app.profile_show(&key)).await;
            match read {
                Ok(Ok(profile)) => RuntimeAssetsInstallReport {
                    status: "completed",
                    payload: Some(profile_to_multica_exchange(&profile)),
                    result: None,
                    error: None,
                },
                Ok(Err(error)) => {
                    failed_install_report(format!("profile `{asset_key}` read failed: {error}"))
                }
                Err(error) => failed_install_report(format!("profile read task failed: {error}")),
            }
        }
    };
    report_install(&client, &runtime_id, &request_id, report).await;
}

/// push-install 工单：交换文档 → 新本地用户角色 → 回报 `installed_name`（新 profile.id）。
///
/// 落库恒为 `create_profile`（新 id 的新用户角色）：与既有角色重名时转换层改名拷贝、
/// `pf-builtin-*`/`is_builtin` 推送落为用户副本——内置角色只读、用户数据不覆盖（ADR-0008 D6）。
async fn handle_push_install<R: Runtime>(
    app: AppHandle<R>,
    client: MulticaClient,
    runtime_id: String,
    request_id: String,
    exchange: MulticaRoleExchange,
) {
    let report = match home_app(&app) {
        None => failed_install_report("app state unavailable".to_string()),
        Some(app) => {
            let install =
                tauri::async_runtime::spawn_blocking(move || -> Result<ProfileEntry, String> {
                    let existing = app
                        .profiles()
                        .map_err(|error| format!("profile scan failed: {error}"))?;
                    let used: BTreeSet<String> = existing
                        .profiles
                        .iter()
                        .map(|profile| profile.name.clone())
                        .collect();
                    let input = multica_exchange_to_profile_input(&exchange, &used)
                        .map_err(|error| error.to_string())?;
                    app.create_profile(input)
                        .map_err(|error| format!("create profile failed: {error}"))
                })
                .await;
            match install {
                Ok(Ok(entry)) => RuntimeAssetsInstallReport {
                    status: "completed",
                    payload: None,
                    result: Some(RuntimeAssetsInstallResult {
                        // id = 资产 key（回查句柄）；展示名 = 实际落库名（改名拷贝时是「xx-2」，
                        // multica 反馈 P3——仅市场名兜底拿不到这个值）。
                        installed_name: entry.id,
                        installed_display_name: entry.name,
                    }),
                    error: None,
                },
                Ok(Err(error)) => failed_install_report(error),
                Err(error) => failed_install_report(format!("install task failed: {error}")),
            }
        }
    };
    report_install(&client, &runtime_id, &request_id, report).await;
}

/// install 端点上报（export/push-install/结构失败共用）；失败记日志，服务端超时兜底。
async fn report_install(
    client: &MulticaClient,
    runtime_id: &str,
    request_id: &str,
    report: RuntimeAssetsInstallReport,
) {
    if let Err(error) = client
        .report_runtime_assets_install_result(runtime_id, request_id, &report)
        .await
    {
        tracing::warn!(
            runtime_id = %runtime_id,
            request_id = %request_id,
            %error,
            "multica runtime-assets install report failed after retries (server timeout will finalize)"
        );
    }
}

fn failed_list_report(error: String) -> RuntimeAssetsListReport {
    RuntimeAssetsListReport {
        status: "failed",
        supported: true,
        assets: Vec::new(),
        error: Some(error),
    }
}

fn failed_install_report(error: String) -> RuntimeAssetsInstallReport {
    RuntimeAssetsInstallReport {
        status: "failed",
        payload: None,
        result: None,
        error: Some(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gold_band::app::ProfileScope;
    use serde_json::json;

    fn detail(action: &str, item_id: &str, payload: serde_json::Value) -> RuntimeAssetsRequest {
        RuntimeAssetsRequest {
            action: action.into(),
            asset_type: "role".into(),
            item_id: item_id.into(),
            payload: Some(payload),
        }
    }

    #[test]
    fn classify_routes_three_work_orders() {
        // list 工单。
        assert!(matches!(
            classify_runtime_assets_request(&detail("list", "", json!({}))),
            Ok(RuntimeAssetsWorkOrder::List)
        ));
        // export 工单：item_id 空 + asset_key payload。
        assert!(matches!(
            classify_runtime_assets_request(&detail(
                "install",
                "",
                json!({"asset_key": "pf-abc"})
            )),
            Ok(RuntimeAssetsWorkOrder::Export { ref asset_key }) if asset_key == "pf-abc"
        ));
        // push-install 工单：item_id 非空 + 角色正文 payload。
        let install = classify_runtime_assets_request(&detail(
            "install",
            "item-uuid",
            json!({
                "name": "pf-role",
                "description": "d",
                "content": "---\nname: pf-role\n---\n\nbody",
                "is_builtin": false
            }),
        ));
        match install {
            Ok(RuntimeAssetsWorkOrder::PushInstall { exchange }) => {
                assert_eq!(exchange.name, "pf-role");
                assert!(exchange.content.starts_with("---\n"));
            }
            other => panic!("expected push-install, got {other:?}"),
        }
    }

    #[test]
    fn classify_rejects_structural_errors() {
        // 未知 action。
        assert!(classify_runtime_assets_request(&detail("reindex", "", json!({}))).is_err());
        // export payload 缺失 / asset_key 空。
        let mut missing = detail("install", "", json!({}));
        missing.payload = None;
        assert!(classify_runtime_assets_request(&missing).is_err());
        assert!(
            classify_runtime_assets_request(&detail("install", "", json!({"asset_key": " "})))
                .is_err()
        );
        // install payload 非角色形态（缺 content）。
        assert!(
            classify_runtime_assets_request(&detail(
                "install",
                "item-uuid",
                json!({"name": "pf-role"})
            ))
            .is_err()
        );
    }

    #[test]
    fn asset_summaries_map_local_identity() {
        // key = profile.id（export 工单按它回取）、type 恒 role、内置标记透传、
        // updated_at 转 RFC3339（multica 反馈 P4）。
        let entry = ProfileEntry {
            id: "pf-xyz".into(),
            name: "评审专家".into(),
            summary: "代码评审".into(),
            summary_source: "代码评审".into(),
            content: "正文".into(),
            dynamic_template: false,
            scope: ProfileScope::User,
            is_built_in: false,
            created_at: "2026-10-31 00:00:00".into(),
            updated_at: "2026-10-31 00:00:00".into(),
            path: String::new(),
        };
        let built_in = ProfileEntry {
            id: "pf-builtin-plan".into(),
            is_built_in: true,
            ..entry.clone()
        };
        let summaries = asset_summaries(&[entry, built_in]);
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].key, "pf-xyz");
        assert_eq!(summaries[0].r#type, "role");
        assert_eq!(summaries[0].name, "评审专家");
        assert!(!summaries[0].is_builtin);
        assert!(summaries[1].is_builtin);
        // RFC3339：T 分隔 + 时区偏移（断言不绑定本机时区）。
        assert!(summaries[0].updated_at.starts_with("2026-10-31T00:00:00"));
        assert_eq!(summaries[0].updated_at.len(), 25);
    }

    #[test]
    fn rfc3339_conversion_handles_formats() {
        // 本地时区格式 → RFC3339：日期时间以 T 分隔、带 ±HH:MM 偏移（共 25 字符，与本机时区无关）。
        let converted = rfc3339_from_local_timestamp("2026-10-31 11:00:00");
        assert!(converted.starts_with("2026-10-31T11:00:00"));
        assert_eq!(converted.len(), 25);
        // 解析失败原样返回（展示降级，不产生错误时间）。
        assert_eq!(
            rfc3339_from_local_timestamp("not a timestamp"),
            "not a timestamp"
        );
        assert_eq!(rfc3339_from_local_timestamp(""), "");
    }
}
