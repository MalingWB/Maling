//! multica runtime-assets 工单（角色/工作流共享通道 B）处理层。
//!
//! 三种工单 × 两类资产（角色设计 `.claude/design/multica-role-management/` §4；工作流设计
//! `.claude/design/multica-workflow-management/码灵对接说明(1).md` §4；multica 网页「共享」组
//! 的上传面板/安装入口遥控本机干活，码灵**纯被动**——唯一主动行为是既有的 15s 心跳认领）：
//! - **list（盘点）**：按详情 `asset_type` 枚举本地资产，只报摘要不报正文（正文仅在
//!   export/install 全量传输）；asset_type 空 = 未分化的全量盘点（role + workflow 逐条带 type）；
//! - **export（提取）**：按 `asset_key`（= 资产 id）读完整资产回传，multica 暂存后自行发布；
//! - **push-install（推送安装）**：把 multica 推来的资产正文落为本地资产，回报实际落库名
//!   （role 恒新用户角色；workflow 同 key 用户模板跳过、builtin id 物化用户副本，均不覆盖）。
//!
//! 格式转换的单一真源在 lib：角色 `gold_band::app::profiles`、工作流 `gold_band::app::workflow_share`；
//! wire 类型在 `client.rs`。本模块只做工单分发与本地读写，与 skill 待办同模型：无状态消费、
//! 上报失败经 client 内网络重试后记日志放弃、服务端 60s running 超时终态化兜底（对迟到/重复
//! 上报幂等）。**不认识的 asset_type 一律 failed 上报**（multica 安装端点对任何 owner 调用方
//! 开放，旧版 daemon 误当 role 处理不是纯理论——工作流对接说明 §6 防御项）。

use std::collections::BTreeSet;

use gold_band::app::{
    MulticaRoleExchange, MulticaWorkflowExchange, ProfileEntry, WorkflowTemplate,
    multica_exchange_to_profile_input, profile_to_multica_exchange,
    workflow_template_to_multica_exchange,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

use crate::multica::client::{
    MulticaClient, RuntimeAssetPayload, RuntimeAssetSummary, RuntimeAssetsExportPayload,
    RuntimeAssetsInstallReport, RuntimeAssetsInstallResult, RuntimeAssetsListReport,
    RuntimeAssetsRequest,
};
use crate::multica::local_skills::home_app;

/// 工单的资产类型常量（multica ADR-0008 runtime kinds）。
const ASSET_TYPE_ROLE: &str = "role";
const ASSET_TYPE_WORKFLOW: &str = "workflow";

/// 被动安装落地事件：push-install 成功落库后 emit，前端按 `assetType` 只重拉对应列表
/// （工作流模板 / 角色）——运行中的前端列表只在会话壳就绪时加载一次，multica 网页的成功
/// toast 在远端，本地此前无任何感知（M5-cr 内网反馈「装完成功但列表不见」）。事件名与
/// web `app-events.ts` 对应；常量模块自持（先例 `multica/upstream.rs`）。
const RUNTIME_ASSET_INSTALLED_EVENT: &str = "gold-band://runtime-asset-installed";

/// 安装事件载荷（wire camelCase，同 `multica/upstream.rs` 的 `UpstreamPullEventVm` 约定）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RuntimeAssetInstalledEventVm {
    asset_type: String,
    installed_display_name: String,
}

/// 安装成功后广播 + INFO 日志（此前成功路径零日志，联调排查只能翻 workflows.json）。
/// emit best-effort：失败不影响工单上报（刷新是增强，不是链路前提）。
fn emit_runtime_asset_installed<R: Runtime>(
    app: &AppHandle<R>,
    request_id: &str,
    asset_type: &str,
    installed_display_name: &str,
) {
    tracing::info!(
        request_id,
        asset_type,
        installed_display_name,
        "multica runtime-asset installed locally"
    );
    let _ = app.emit(
        RUNTIME_ASSET_INSTALLED_EVENT,
        RuntimeAssetInstalledEventVm {
            asset_type: asset_type.to_string(),
            installed_display_name: installed_display_name.to_string(),
        },
    );
}

/// 详情 → 工单分类结果（纯函数，可单测；action/asset_type 均以详情为权威，ack 里的 action
/// 只是建议）。
#[derive(Debug)]
pub(crate) enum RuntimeAssetsWorkOrder {
    /// 盘点：按详情 asset_type 枚举摘要。
    List,
    /// 提取角色：`item_id` 空 + payload `{asset_key}`；按 key 回读完整角色。
    Export { asset_key: String },
    /// 推送安装角色：`item_id` 非空 + payload 即角色正文。
    PushInstall { exchange: MulticaRoleExchange },
    /// 提取工作流：payload 同 `{asset_key}` 形；按 key 回读模板 envelope（剥 modelBindings）。
    ExportWorkflow { asset_key: String },
    /// 推送安装工作流：payload 即模板 envelope（服务端已剥 modelBindings）。
    PushInstallWorkflow { exchange: MulticaWorkflowExchange },
}

/// 把工单详情分类为五种工单之一；结构非法（payload 缺失/不合规、action/asset_type 未知）
/// 返回错误文案（调用方按 failed 上报终态化，不静默吞掉——网页端要能看到失败原因）。
///
/// asset_type 空 → 按角色处理（角色通道先于 asset_type 字段上线，旧 server 不发该字段时
/// 下发的只会是角色工单；新类型工作流/未来类型恒显式携带）。未知 asset_type 显式拒绝，
/// **绝不回落角色**（防御：安装端点对任何 owner 调用方开放，误当 role 处理会产生脏写入）。
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
            let export = detail.item_id.trim().is_empty();
            match detail.asset_type.as_str() {
                "" | ASSET_TYPE_ROLE => classify_role_install(payload, export),
                ASSET_TYPE_WORKFLOW => classify_workflow_install(payload, export),
                other => Err(format!(
                    "unsupported runtime-assets asset type: {other}"
                )),
            }
        }
        other => Err(format!("unsupported runtime-assets action: {other}")),
    }
}

/// 角色形态 install 工单（export：`{asset_key}`；push-install：角色交换文档）。
fn classify_role_install(
    payload: serde_json::Value,
    export: bool,
) -> Result<RuntimeAssetsWorkOrder, String> {
    if export {
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

/// 工作流形态 install 工单（export：`{asset_key}`；push-install：模板 envelope）。
fn classify_workflow_install(
    payload: serde_json::Value,
    export: bool,
) -> Result<RuntimeAssetsWorkOrder, String> {
    if export {
        let payload: RuntimeAssetsExportPayload = serde_json::from_value(payload)
            .map_err(|error| format!("workflow export payload invalid: {error}"))?;
        if payload.asset_key.trim().is_empty() {
            return Err("workflow export payload asset_key empty".to_string());
        }
        Ok(RuntimeAssetsWorkOrder::ExportWorkflow {
            asset_key: payload.asset_key,
        })
    } else {
        let exchange: MulticaWorkflowExchange = serde_json::from_value(payload)
            .map_err(|error| format!("workflow install payload invalid: {error}"))?;
        Ok(RuntimeAssetsWorkOrder::PushInstallWorkflow { exchange })
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

/// 本地工作流模板列表 → list 工单上报摘要（纯映射；`key` = 模板 id，export 工单按它回取；
/// builtin 正常上报带徽标——multaca 目录允许发布 builtin 副本，ADR-0008 D6）。
///
/// description 恒空：本地模板没有描述字段（web 弹窗对空描述有展示容忍）。
pub(crate) fn workflow_asset_summaries(
    templates: &[WorkflowTemplate],
) -> Vec<RuntimeAssetSummary> {
    templates
        .iter()
        .map(|template| RuntimeAssetSummary {
            key: template.id.clone(),
            r#type: ASSET_TYPE_WORKFLOW,
            name: template.name.clone(),
            description: String::new(),
            is_builtin: template.is_built_in,
            updated_at: rfc3339_from_epoch_timestamp(&template.updated_at),
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

/// 工作流模板时间戳（`<unix 秒>Z`，`ids::now_rfc3339_like` 格式，绝对时刻无时区歧义）
/// → RFC3339（UTC `Z` 后缀）。
///
/// 与角色同降级策略：解析失败（历史文件格式漂移）原样返回，好于错误时间。
fn rfc3339_from_epoch_timestamp(timestamp: &str) -> String {
    timestamp
        .strip_suffix('Z')
        .and_then(|secs| secs.parse::<i64>().ok())
        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
        .map(|datetime| datetime.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
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
        Ok(RuntimeAssetsWorkOrder::ExportWorkflow { asset_key }) => {
            handle_export_workflow(app, client, runtime_id, request_id, asset_key).await
        }
        Ok(RuntimeAssetsWorkOrder::PushInstallWorkflow { exchange }) => {
            handle_push_install_workflow(app, client, runtime_id, request_id, exchange).await
        }
        Err(error) => {
            // 结构性失败（payload 非法/action/asset_type 未知）：failed 上报终态化，网页端
            // 可见原因。classify 只对 install 形态工单可能失败，上报走 install 端点。
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

/// list 工单：按详情 `asset_type` 枚举本地资产 → 摘要上报（正文不上传）。
///
/// - `role` → 只报角色；`workflow` → 只报模板（web 弹窗按传入类型渲染，**必须只回该类型**，
///   混入另一类会显示错乱——工作流对接说明 §4.2）；
/// - 空 → role + workflow 一并回报，逐条带 `type`（未分化盘点的建议形态）；
/// - 其它类型 → `supported: false` + 空 assets，如实回报能力（server 据此不再下发该类型工单）。
async fn handle_list<R: Runtime>(
    app: AppHandle<R>,
    client: MulticaClient,
    runtime_id: String,
    request_id: String,
    asset_type: String,
) {
    let want_role = asset_type.is_empty() || asset_type == ASSET_TYPE_ROLE;
    let want_workflow = asset_type.is_empty() || asset_type == ASSET_TYPE_WORKFLOW;
    let report = match home_app(&app) {
        None => failed_list_report("app state unavailable".to_string()),
        Some(app) => {
            // 目录/store 扫描是阻塞 FS（角色十级文件 <10ms、模板单 JSON 文件），放
            // spawn_blocking 不占 async worker；只读请求的类型，不引入另一类的失败耦合。
            let scan = tauri::async_runtime::spawn_blocking(move || {
                let mut assets = Vec::new();
                if want_role {
                    let list = app
                        .profiles()
                        .map_err(|error| format!("profile scan failed: {error}"))?;
                    assets.extend(asset_summaries(&list.profiles));
                }
                if want_workflow {
                    let store = app
                        .workflow_templates()
                        .map_err(|error| format!("workflow template scan failed: {error}"))?;
                    assets.extend(workflow_asset_summaries(&store.templates));
                }
                Ok::<_, String>(assets)
            })
            .await;
            match scan {
                Ok(Ok(assets)) => RuntimeAssetsListReport {
                    status: "completed",
                    supported: want_role || want_workflow,
                    assets,
                    error: None,
                },
                Ok(Err(error)) => failed_list_report(error),
                Err(error) => failed_list_report(format!("asset scan task failed: {error}")),
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

/// export 工单（角色）：按 asset_key（profile.id）读完整角色 → 交换文档回传（multica 暂存后
/// 自行发布）。
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
                    payload: Some(RuntimeAssetPayload::Role(
                        profile_to_multica_exchange(&profile),
                    )),
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

/// push-install 工单（角色）：交换文档 → 新本地用户角色 → 回报 `installed_name`（新 profile.id）。
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
        Some(home) => {
            let install =
                tauri::async_runtime::spawn_blocking(move || -> Result<ProfileEntry, String> {
                    let existing = home
                        .profiles()
                        .map_err(|error| format!("profile scan failed: {error}"))?;
                    let used: BTreeSet<String> = existing
                        .profiles
                        .iter()
                        .map(|profile| profile.name.clone())
                        .collect();
                    let input = multica_exchange_to_profile_input(&exchange, &used)
                        .map_err(|error| error.to_string())?;
                    home.create_profile(input)
                        .map_err(|error| format!("create profile failed: {error}"))
                })
                .await;
            match install {
                Ok(Ok(entry)) => {
                    emit_runtime_asset_installed(
                        &app,
                        &request_id,
                        ASSET_TYPE_ROLE,
                        &entry.name,
                    );
                    RuntimeAssetsInstallReport {
                        status: "completed",
                        payload: None,
                        result: Some(RuntimeAssetsInstallResult {
                            // id = 资产 key（回查句柄）；展示名 = 实际落库名（改名拷贝时是「xx-2」，
                            // multica 反馈 P3——仅市场名兜底拿不到这个值）。
                            installed_name: entry.id,
                            installed_display_name: entry.name,
                        }),
                        error: None,
                    }
                }
                Ok(Err(error)) => failed_install_report(error),
                Err(error) => failed_install_report(format!("install task failed: {error}")),
            }
        }
    };
    report_install(&client, &runtime_id, &request_id, report).await;
}

/// export 工单（工作流）：按 asset_key（模板 id）读模板 → envelope 回传（multica 暂存后自行
/// 发布）。modelBindings 剥离在 envelope 构造层成立（结构无该字段）。
async fn handle_export_workflow<R: Runtime>(
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
            let read = tauri::async_runtime::spawn_blocking(move || {
                app.workflow_templates()
                    .map_err(|error| format!("workflow template scan failed: {error}"))
                    .and_then(|store| {
                        store
                            .templates
                            .iter()
                            .find(|template| template.id == key)
                            .map(workflow_template_to_multica_exchange)
                            .ok_or_else(|| format!("workflow template `{key}` not found"))
                    })
            })
            .await;
            match read {
                Ok(Ok(exchange)) => RuntimeAssetsInstallReport {
                    status: "completed",
                    payload: Some(RuntimeAssetPayload::Workflow(exchange)),
                    result: None,
                    error: None,
                },
                Ok(Err(error)) => {
                    failed_install_report(format!("workflow template `{asset_key}` read failed: {error}"))
                }
                Err(error) => {
                    failed_install_report(format!("workflow template read task failed: {error}"))
                }
            }
        }
    };
    report_install(&client, &runtime_id, &request_id, report).await;
}

/// push-install 工单（工作流）：模板 envelope → 本地落库 → 回报实际落库 id/名。
///
/// 落库语义（skip-if-exists / builtin 副本 / id 保留）在 lib
/// `App::install_workflow_template_from_multica` 单一真源；此处只做工单编排。
async fn handle_push_install_workflow<R: Runtime>(
    app: AppHandle<R>,
    client: MulticaClient,
    runtime_id: String,
    request_id: String,
    exchange: MulticaWorkflowExchange,
) {
    let report = match home_app(&app) {
        None => failed_install_report("app state unavailable".to_string()),
        Some(home) => {
            let install = tauri::async_runtime::spawn_blocking(move || {
                home.install_workflow_template_from_multica(&exchange)
                    .map_err(|error| format!("install workflow template failed: {error}"))
            })
            .await;
            match install {
                Ok(Ok(template)) => {
                    emit_runtime_asset_installed(
                        &app,
                        &request_id,
                        ASSET_TYPE_WORKFLOW,
                        &template.name,
                    );
                    RuntimeAssetsInstallReport {
                        status: "completed",
                        payload: None,
                        result: Some(RuntimeAssetsInstallResult {
                            // 与角色同约定：id = 资产 key（跳过时是既有模板 id），展示名 = 实际
                            // 落库名（builtin 副本派生 id 场景用户可读）。
                            installed_name: template.id,
                            installed_display_name: template.name,
                        }),
                        error: None,
                    }
                }
                Ok(Err(error)) => failed_install_report(error),
                Err(error) => {
                    failed_install_report(format!("workflow install task failed: {error}"))
                }
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

    #[test]
    fn installed_event_serializes_camel_case_wire() {
        // 跨端契约（M5-cr 被动安装刷新）：前端 `web/tests/runtime-asset-installed-event.test.ts`
        // 锁同一事件名与形状；Rust 侧键名漂移（如丢失 rename_all）此处先红。
        let vm = RuntimeAssetInstalledEventVm {
            asset_type: "workflow".to_string(),
            installed_display_name: "我的流水线".to_string(),
        };
        let wire = serde_json::to_value(&vm).unwrap();
        assert_eq!(wire["assetType"], "workflow");
        assert_eq!(wire["installedDisplayName"], "我的流水线");
        assert_eq!(wire.as_object().unwrap().len(), 2);
    }

    fn detail(action: &str, item_id: &str, payload: serde_json::Value) -> RuntimeAssetsRequest {
        detail_typed("role", action, item_id, payload)
    }

    fn detail_typed(
        asset_type: &str,
        action: &str,
        item_id: &str,
        payload: serde_json::Value,
    ) -> RuntimeAssetsRequest {
        RuntimeAssetsRequest {
            action: action.into(),
            asset_type: asset_type.into(),
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
    fn classify_routes_workflow_work_orders() {
        // export 工单（workflow）：payload 与角色同形 {asset_key}，按 asset_type 分派到工作流分支。
        assert!(matches!(
            classify_runtime_assets_request(&detail_typed(
                "workflow",
                "install",
                "",
                json!({"asset_key": "my-pipeline"})
            )),
            Ok(RuntimeAssetsWorkOrder::ExportWorkflow { ref asset_key }) if asset_key == "my-pipeline"
        ));
        // push-install 工单（workflow）：item_id 非空 + 模板 envelope payload。
        let install = classify_runtime_assets_request(&detail_typed(
            "workflow",
            "install",
            "item-uuid",
            json!({
                "id": "my-pipeline",
                "name": "我的流水线",
                "isBuiltIn": false,
                "workflow": {
                    "version": "0.1", "id": "task-workflow", "entry": "dev",
                    "nodes": [{ "type": "worker", "id": "dev", "profile": "pf-builtin-dev",
                                "provider": null, "goal": null }],
                    "edges": [{ "from": "dev", "to": "$end", "on": "success" }]
                }
            }),
        ));
        match install {
            Ok(RuntimeAssetsWorkOrder::PushInstallWorkflow { exchange }) => {
                assert_eq!(exchange.id, "my-pipeline");
                assert!(!exchange.is_built_in);
                assert_eq!(exchange.workflow.nodes.len(), 1);
            }
            other => panic!("expected push-install-workflow, got {other:?}"),
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
    fn classify_rejects_unknown_asset_type_and_tolerates_legacy_empty() {
        // 未知 asset_type：显式拒绝（failed 上报），绝不误当 role 处理——安装端点对任何
        // owner 调用方开放，误分类会产生脏写入（工作流对接说明 §6 防御项）。
        let error = classify_runtime_assets_request(&detail_typed(
            "agent",
            "install",
            "item-uuid",
            json!({"anything": "shapes"}),
        ))
        .unwrap_err();
        assert!(error.contains("unsupported runtime-assets asset type: agent"));
        // asset_type 空：旧 server 容忍，回落角色分支（角色通道先于该字段上线）。
        assert!(matches!(
            classify_runtime_assets_request(&detail_typed(
                "",
                "install",
                "",
                json!({"asset_key": "pf-abc"})
            )),
            Ok(RuntimeAssetsWorkOrder::Export { .. })
        ));
        // workflow envelope 误投 role 类型：缺 content 解析失败（failed，不落库）。
        assert!(
            classify_runtime_assets_request(&detail_typed(
                "role",
                "install",
                "item-uuid",
                json!({"id": "my-pipeline", "name": "n", "isBuiltIn": false})
            ))
            .is_err()
        );
        // workflow 分支的 export asset_key 空：同样结构性拒绝。
        assert!(
            classify_runtime_assets_request(&detail_typed(
                "workflow",
                "install",
                "",
                json!({"asset_key": " "})
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
        // epoch 秒格式（模板时间戳）→ RFC3339 UTC：Z 后缀、T 分隔（绝对时刻无时区歧义）。
        // 1790000000 = 2026-09-21T14:13:20Z（与本机时区无关的定值断言）。
        assert_eq!(
            rfc3339_from_epoch_timestamp("1790000000Z"),
            "2026-09-21T14:13:20Z"
        );
        // 解析失败原样返回（与角色同降级策略）。
        assert_eq!(
            rfc3339_from_epoch_timestamp("2026-10-31 00:00:00"),
            "2026-10-31 00:00:00"
        );
        assert_eq!(rfc3339_from_epoch_timestamp(""), "");
    }

    #[test]
    fn workflow_asset_summaries_map_local_identity() {
        // key = 模板 id（export 工单按它回取）、type 恒 workflow、description 恒空（模板无
        // 描述字段）、builtin 正常上报带徽标（ADR-0008 D6）、updated_at epoch → RFC3339。
        let template = WorkflowTemplate {
            id: "my-pipeline".into(),
            name: "我的流水线".into(),
            is_built_in: false,
            optional_entry_stage: None,
            workflow: sample_workflow_dsl(),
            model_bindings: Default::default(),
            created_at: "1790000000Z".into(),
            updated_at: "1790000000Z".into(),
        };
        let built_in = WorkflowTemplate {
            id: "default".into(),
            is_built_in: true,
            ..template.clone()
        };
        let summaries = workflow_asset_summaries(&[template, built_in]);
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].key, "my-pipeline");
        assert_eq!(summaries[0].r#type, "workflow");
        assert_eq!(summaries[0].name, "我的流水线");
        assert_eq!(summaries[0].description, "");
        assert!(!summaries[0].is_builtin);
        assert!(summaries[1].is_builtin);
        assert_eq!(summaries[0].updated_at, "2026-09-21T14:13:20Z");
    }

    fn sample_workflow_dsl() -> gold_band::dsl::WorkflowDsl {
        use gold_band::dsl::{EdgeDsl, EdgeOutcome, END_NODE, NodeDsl, WorkerNode};
        gold_band::dsl::WorkflowDsl {
            version: "0.1".to_string(),
            id: "workflow-sample".to_string(),
            entry: "dev".to_string(),
            control: Default::default(),
            nodes: vec![NodeDsl::Worker(WorkerNode {
                id: "dev".to_string(),
                execution_slot_id: None,
                provider: Some("claude-acp".to_string()),
                model: None,
                profile: Some("pf-builtin-dev".to_string()),
                goal: None,
                output: None,
                success_condition: None,
                permission_mode: None,
                auto_accept: false,
                config_options: Default::default(),
                manual_check: None,
                prompt_envelope: Default::default(),
            })],
            edges: vec![EdgeDsl {
                from: "dev".to_string(),
                to: END_NODE.to_string(),
                on: EdgeOutcome::Success,
                session: None,
                new_round_entry: None,
            }],
        }
    }
}
