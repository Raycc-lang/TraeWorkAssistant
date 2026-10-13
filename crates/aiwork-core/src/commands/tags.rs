//! 账号标签体系（shared-groups-tags-design.md P1，需求③）：
//! - 标签定义存 `tags` 行文档表（Tag JSON，id 前缀 tag_）；绑定存 `account_tags`
//!   行文档表（pk = 跨池全局唯一 uid，data = [tag_id, ...] 多对多，D3 独立表不塞
//!   账号结构）；
//! - 语义：分组 = 单选归属；标签 = 多选标注，两者正交；
//! - 删除标签时清理 account_tags / api_keys.allowed_tags / api_pool.*_tag_ids
//!   引用 + 热重载（顺序对齐分组删除：先清绑定、再删定义、最后清引用）。
//!
//! docker 适配：`#[tauri::command]` 壳删除（经 cmd_bridge 白名单分发），
//! `State<AppState>` → `&AppState`；热重载改 `reload_pools_after_change`。

use std::collections::HashMap;

use serde::Serialize;

use crate::models::Tag;
use crate::state::AppState;

/// 标签视图（对齐 GroupView：uids 从 account_tags 反查，跨池全局）
#[derive(Serialize, Clone)]
pub struct TagView {
    pub id: String,
    pub name: String,
    pub color: String,
    pub order: i32,
    pub count: usize,
    pub uids: Vec<String>,
}

fn defs_load(state: &AppState) -> Vec<Tag> {
    crate::store::docs::tags_load(&crate::store::db(&state.data_dir))
}

fn defs_save(state: &AppState, defs: &[Tag]) -> Result<(), String> {
    crate::store::docs::tags_save(&crate::store::db(&state.data_dir), defs)
}

/// 全局重名校验（trim/非空/唯一；exclude_id 供 update 排除自身）
fn validate_name(defs: &[Tag], name: &str, exclude_id: Option<&str>) -> Result<String, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("标签名不能为空".into());
    }
    if defs
        .iter()
        .any(|t| t.name == name && Some(t.id.as_str()) != exclude_id)
    {
        return Err(format!("标签「{name}」已存在"));
    }
    Ok(name)
}

/// 孤儿绑定检测的存活 uid 全集（审查 P5）：Trae accounts 表 user_id + WB/Qoder 池
/// 账号 id（行文档 data JSON）。任一源表读失败时返回 None，调用方跳过本轮自愈
/// （避免读库故障时把全部绑定误当孤儿清理）
fn known_uids(state: &AppState) -> Option<std::collections::HashSet<String>> {
    let store = crate::store::db(&state.data_dir);
    let mut known = std::collections::HashSet::new();
    // Trae 侧直读 accounts 表 user_id 列（审查 B4：accounts_load 与 vault 凭据回填
    // 内部 fail-open，且此处只需账号存在性、无需凭据；读失败显式跳过本轮自愈并记
    // 日志——静默空集会把全部 Trae 绑定误当孤儿清理）
    match store.with_conn(|c| {
        let mut stmt = c.prepare("SELECT user_id FROM accounts")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, Option<String>>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }) {
        Ok(rows) => {
            for uid in rows.into_iter().flatten() {
                if !uid.is_empty() {
                    known.insert(uid);
                }
            }
        }
        Err(e) => {
            crate::fs_utils::app_log(
                &state.data_dir,
                &format!("tags: 孤儿自愈跳过本轮（Trae 账号源读失败，防误删）: {e}"),
            );
            return None;
        }
    }
    for table in ["wb_accounts", "qoder_accounts"] {
        match store.rows_all(table) {
            Ok(rows) => {
                for (_pk, data) in rows {
                    if let Some(id) = data.get("id").and_then(|v| v.as_str()) {
                        known.insert(id.to_string());
                    }
                }
            }
            Err(_) => return None,
        }
    }
    Some(known)
}

pub fn tags_list(state: &AppState) -> Vec<TagView> {
    let store = crate::store::db(&state.data_dir);
    let mut uid_by_tag: HashMap<String, Vec<String>> = HashMap::new();
    for (uid, ids) in crate::store::docs::account_tags_map(&store) {
        for t in ids {
            uid_by_tag.entry(t).or_default().push(uid.clone());
        }
    }
    // 孤儿绑定自愈（审查 P5）：指向已不存在账号的绑定行顺带清理 + 记日志
    // （账号删除路径理论上会清绑定，历史数据/并发窗口内可能残留）。源表读失败
    // 跳过本轮自愈；三池账号清单全空（读取降级成空与真空库不可区分）时同样跳过，
    // 防把全部绑定当孤儿整表误删；单条清理失败仅记日志不影响列表；无孤儿时零写开销
    if let Some(known) = known_uids(state).filter(|known| !known.is_empty()) {
        let orphans: Vec<String> = uid_by_tag
            .values()
            .flatten()
            .cloned()
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .filter(|uid| !known.contains(uid))
            .collect();
        let mut removed = 0usize;
        for uid in &orphans {
            match crate::store::docs::account_tags_remove_account(&store, uid) {
                Ok(()) => {
                    removed += 1;
                    crate::fs_utils::app_log(
                        &state.data_dir,
                        &format!("tags_list: 清理孤儿标签绑定（账号已不存在）: {uid}"),
                    );
                    for uids in uid_by_tag.values_mut() {
                        uids.retain(|u| u != uid);
                    }
                }
                Err(e) => crate::fs_utils::app_log(
                    &state.data_dir,
                    &format!("tags_list: 孤儿标签绑定清理失败（不影响列表）: {uid}: {e}"),
                ),
            }
        }
        if removed > 0 {
            crate::api_server::org_index::invalidate(&state.data_dir);
        }
    }
    defs_load(state)
        .into_iter()
        .map(|t| {
            let uids = uid_by_tag.remove(&t.id).unwrap_or_default();
            let count = uids.len();
            TagView { id: t.id, name: t.name, color: t.color, order: t.order, count, uids }
        })
        .collect()
}

fn create_inner(state: &AppState, name: String, color: String) -> Result<String, String> {
    // 持 group_defs_lock（与分组定义同一把组织定义互斥）：tags 亦为整删全写模型，
    // 「load→改→save」须整体互斥——仅靠 docs::tags_save 内部锁只串行化写库，
    // 两并发创建均在锁外 load 同一旧快照、后写覆盖先写丢新标签（与 B1 修的
    // groups 丢写完全同构）
    let _guard = state.group_defs_lock.lock().unwrap_or_else(|e| e.into_inner());
    let mut defs = defs_load(state);
    let name = validate_name(&defs, &name, None)?;
    let rand = uuid::Uuid::new_v4().simple().to_string();
    let id = format!("tag_{}{}", chrono::Local::now().timestamp_millis(), &rand[..4]);
    let order = (defs.len() as i32) + 1;
    defs.push(Tag { id: id.clone(), name, color, order });
    defs_save(state, &defs)?;
    Ok(id)
}

pub fn tags_create(state: &AppState, name: String, color: String) -> Result<String, String> {
    create_inner(state, name, color)
}

fn update_inner(
    state: &AppState,
    id: String,
    name: Option<String>,
    color: Option<String>,
    order: Option<i32>,
) -> Result<(), String> {
    // 持 group_defs_lock：RMW 整体互斥（同 create_inner）
    let _guard = state.group_defs_lock.lock().unwrap_or_else(|e| e.into_inner());
    let mut defs = defs_load(state);
    let new_name = match name.as_deref() {
        Some(n) => Some(validate_name(&defs, n, Some(&id))?),
        None => None,
    };
    let t = defs.iter_mut().find(|t| t.id == id).ok_or("标签不存在")?;
    if let Some(n) = new_name {
        t.name = n;
    }
    if let Some(c) = color {
        t.color = c;
    }
    if let Some(o) = order {
        t.order = o;
    }
    defs_save(state, &defs)
}

pub fn tags_update(
    state: &AppState,
    id: String,
    name: Option<String>,
    color: Option<String>,
    order: Option<i32>,
) -> Result<(), String> {
    update_inner(state, id, name, color, order)
}

/// 删除标签：先清绑定 → 再删定义 → 最后清 api_keys / api_pool 引用 + 热重载。
/// （反序在中途失败会产生「定义已删、引用悬空」；本序最坏情况是绑定清理成功但
/// 定义残留，可重试删除，无悬空引用。）
fn delete_inner(state: &AppState, id: &str) -> Result<(), String> {
    let store = crate::store::db(&state.data_dir);
    // ①清绑定 + ②删定义 纳入同一 group_defs_lock 临界区：绑定恢复入口
    // （account_tags_set_inner）在同一把锁内校验 tag id 存在性——两段分离时，
    // 并发 set 在「绑定已清、定义未删」窗口通过校验写回已删标签 → 幽灵绑定复活。
    // 锁序：group_defs_lock 为叶子锁，临界区内不再嵌套其他锁
    {
        let _guard = state.group_defs_lock.lock().unwrap_or_else(|e| e.into_inner());
        // ① 清绑定
        crate::store::docs::account_tags_remove_tag(&store, id)?;
        // ② 删定义
        let mut defs = defs_load(state);
        defs.retain(|t| t.id != id);
        defs_save(state, &defs)?;
    }
    // ③ 清引用：api_keys.allowed_tags + api_pool 三池标签筛选
    // Key 引用清理走锁内原子修改（防与并发 UI 保存互覆约束字段）。
    // 审查修复：modify 返回的 bool（是否已变更）此前被丢弃，内部如需整体回滚
    // 无从得知；改为显式接收并记录，避免「静默丢弃状态」的一贯隐患
    let keys_changed = crate::api_server::api_keys::modify(&state.data_dir, |keys_file| {
        let mut changed = false;
        for k in keys_file.keys.iter_mut() {
            if k.allowed_tags.iter().any(|t| t == id) {
                k.allowed_tags.retain(|t| t != id);
                changed = true;
            }
        }
        changed
    });
    if keys_changed {
        crate::fs_utils::app_log(
            &state.data_dir,
            &format!("删除标签 {id}：已清理 api_keys.allowed_tags 引用"),
        );
    }
    // 池筛选引用读改写基线统一走迁移入口（对齐上游 P1 审查修复，同 shared_groups；
    // 审查 B1：api_pool 整文档覆盖写互斥——锁内完成读改写+落盘，热重载放锁外）
    let pool_changed = {
        let _pool_cfg = crate::api_server::runtime::api_pool_cfg_lock();
        let mut pool_file = crate::api_server::runtime::load_pool_file(&state.data_dir);
        let mut pool_changed = false;
        for ids in [
            &mut pool_file.trae_tag_ids,
            &mut pool_file.wb_tag_ids,
            &mut pool_file.qoder_tag_ids,
        ] {
            if ids.iter().any(|t| t == id) {
                ids.retain(|t| t != id);
                pool_changed = true;
            }
        }
        if pool_changed {
            store.kv_set("api_pool", &pool_file)?;
        }
        pool_changed
    };
    if pool_changed {
        crate::api_server::runtime::reload_pools_after_change(state);
    }
    // org 展开索引失效（account_tags 变化）
    crate::api_server::org_index::invalidate(&state.data_dir);
    Ok(())
}

pub fn tags_delete(state: &AppState, id: String) -> Result<(), String> {
    delete_inner(state, &id)
}

/// 全量覆盖式设置某账号的标签绑定（前端多选框一次性提交）。
/// 校验 tag_id 存在；uid 跨池全局唯一（Trae 数字 / wb- / qd- 前缀）。
/// uid 存在性由调用方负责（前端仅从各池账号列表取值）；账号删除路径经
/// account_tags_remove_account 清理绑定，不会产生孤儿行。
pub(crate) fn account_tags_set_inner(
    state: &AppState,
    uid: &str,
    tag_ids: &[String],
) -> Result<(), String> {
    let store = crate::store::db(&state.data_dir);
    // tag_id 必须存在（防幽灵标签）。校验与写入纳入同一 group_defs_lock 临界区：
    // 锁外校验存在 TOCTOU——校验后、写入前标签可被并发删除（delete_inner 同锁
    // 清绑定），写入即复活幽灵绑定
    let _guard = state.group_defs_lock.lock().unwrap_or_else(|e| e.into_inner());
    let defs = defs_load(state);
    for t in tag_ids {
        if !defs.iter().any(|d| &d.id == t) {
            return Err(format!("标签不存在: {t}"));
        }
    }
    crate::store::docs::account_tags_set(&store, uid, tag_ids)?;
    // 联动：账号标签变化影响调度池标签筛选（api_pool.*_tag_ids）与 Key 约束展开
    crate::api_server::org_index::invalidate(&state.data_dir);
    Ok(())
}

pub fn account_tags_set(
    state: &AppState,
    uid: String,
    tag_ids: Vec<String>,
) -> Result<(), String> {
    account_tags_set_inner(state, &uid, &tag_ids)?;
    // 调度池热重载（api_pool.*_tag_ids 按账号标签过滤入池账号）
    crate::api_server::runtime::reload_pools_after_change(state);
    Ok(())
}

/// 一次返回全量 uid → tag_ids 映射（各池账号列表组装联表用，避免 N+1 invoke）
pub fn account_tags_map(state: &AppState) -> HashMap<String, Vec<String>> {
    crate::store::docs::account_tags_map(&crate::store::db(&state.data_dir))
}

/// 导入条目的 tags 字段收集（P2 需求③，qoder/workbuddy/Trae 三处导入共享）：
/// 仅保留本机已存在的 tag id（未知 id 计入 ignored，不自动创建标签，避免导入方
/// 标签体系污染本机）；final_id = 原位更新保留的原 id 或新增 id。
/// 纯内存过滤不触库：调用方在主体落库成功后统一写 account_tags 表
pub fn collect_import_tags(
    entry: &serde_json::Value,
    final_id: &str,
    tag_defs: &[crate::models::Tag],
    out: &mut Vec<(String, Vec<String>)>,
    ignored: &mut usize,
) {
    let Some(arr) = entry.get("tags").and_then(serde_json::Value::as_array) else {
        return;
    };
    let valid: Vec<String> = arr
        .iter()
        .filter_map(|t| t.as_str())
        .filter(|t| {
            if tag_defs.iter().any(|d| d.id == **t) {
                true
            } else {
                *ignored += 1;
                false
            }
        })
        .map(String::from)
        .collect();
    if !valid.is_empty() {
        out.push((final_id.to_string(), valid));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_state(tag: &str) -> (std::path::PathBuf, AppState) {
        let d = std::env::temp_dir().join(format!("twa_tags_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let state = AppState {
            data_dir: d.clone(),
            jwt_refresh_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
            qoder_pool_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
            wb_pool_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
            group_defs_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
            events: std::sync::Arc::new(std::sync::Mutex::new(None)),
        };
        (d, state)
    }

    #[test]
    fn tags_crud_and_account_bindings() {
        let (dir, state) = tmp_state("crud");
        let t1 = create_inner(&state, "高积分".into(), "#0f0".into()).unwrap();
        assert!(t1.starts_with("tag_"));
        assert!(create_inner(&state, "高积分".into(), "#f00".into()).is_err(), "重名拒绝");
        update_inner(&state, t1.clone(), Some("优质".into()), None, None).unwrap();

        // 绑定：跨池 uid（Trae 数字 + wb- + qd-）
        account_tags_set_inner(&state, "10001", &[t1.clone()]).unwrap();
        account_tags_set_inner(&state, "wb-abc", &[t1.clone()]).unwrap();
        let map = crate::store::docs::account_tags_map(&crate::store::db(&state.data_dir));
        assert_eq!(map.get("10001").unwrap(), &vec![t1.clone()]);
        assert_eq!(map.get("wb-abc").unwrap(), &vec![t1.clone()]);
        // 全量覆盖语义：二次设置替换
        account_tags_set_inner(&state, "10001", &[]).unwrap();
        let map = crate::store::docs::account_tags_map(&crate::store::db(&state.data_dir));
        assert!(map.get("10001").is_none() || map["10001"].is_empty());

        // 幽灵标签拒绝
        assert!(account_tags_set_inner(&state, "10001", &["tag_none".into()]).is_err());

        // 删除标签：绑定清理
        account_tags_set_inner(&state, "wb-abc", &[t1.clone()]).unwrap();
        delete_inner(&state, &t1).unwrap();
        let map = crate::store::docs::account_tags_map(&crate::store::db(&state.data_dir));
        assert!(map.get("wb-abc").map(|v| v.is_empty()).unwrap_or(true), "绑定应被清理");
        assert!(defs_load(&state).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
