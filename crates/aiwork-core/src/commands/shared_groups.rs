//! 跨应用共享分组（shared-groups-tags-design.md P0，需求①）：
//! - 分组「定义」统一收敛到既有 `groups` 行文档表（每行一个 Group JSON，id 前缀
//!   统一 `shg_`，启动迁移保证存量全量重定向）；「成员关系」保持各池现状（D1）：
//!   Trae 走 group_members 映射表，WB/Qoder 走账号记录 group_id 字段；
//! - 三池既有分组命令保留旧名作薄封装（前端零改动），内部转发统一实现；
//! - 删除分组时按「先回落成员 → 再删定义 → 最后清引用」顺序（对齐
//!   qoder_groups_remove 审查结论），并联动清理 api_pool / api_keys 引用 + 热重载。
//!
//! docker 适配：`#[tauri::command]` 壳删除（经 cmd_bridge 白名单分发），
//! `State<AppState>` → `&AppState`；热重载改 `reload_pools_after_change`
//! （server 单体网关 OnceLock 注册，未运行时静默跳过）。

use crate::models::Group;
use crate::state::AppState;

use super::accounts::GroupView;

// ── 内部工具 ────────────────────────────────────────────────────────────────

pub(crate) fn defs_load(state: &AppState) -> Vec<Group> {
    crate::store::docs::group_defs_load(&crate::store::db(&state.data_dir))
}

fn defs_save(state: &AppState, defs: &[Group]) -> Result<(), String> {
    crate::store::docs::group_defs_save(&crate::store::db(&state.data_dir), defs)
}

/// 全局重名校验（推广 qoder validate_name；trim/非空/共享定义内唯一）。
/// exclude_id 供 update 排除自身；返回 trim 后的名称
pub(crate) fn validate_name(
    defs: &[Group],
    name: &str,
    exclude_id: Option<&str>,
) -> Result<String, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("分组名不能为空".into());
    }
    if defs
        .iter()
        .any(|g| g.name == name && Some(g.id.as_str()) != exclude_id)
    {
        return Err(format!("分组「{name}」已存在"));
    }
    Ok(name)
}

// ── 列表 ────────────────────────────────────────────────────────────────────

/// 共享分组列表：defs 全局一份；uids = 三池成员并集（trae 数字 uid + wb- + qd-，
/// 命名空间天然不冲突），count = uids.len()。供 API Key 分组绑定等跨池消费方使用。
/// 三池各一次整表扫描构建 gid → uids（审查 B6：禁止每分组重读三池形成 N+1）。
pub fn shared_groups_list(state: &AppState) -> Result<Vec<GroupView>, String> {
    let defs = defs_load(state);
    let store = crate::store::db(&state.data_dir);
    let mut uids_by_gid: std::collections::HashMap<String, Vec<String>> = Default::default();
    // Trae：group_members 表一次全量。
    // 审查修复：原为 `if let Ok(rows)` 静默吞错——读失败时该池成员被当成「零成员」，
    // 计数偏小的列表会让用户误判「空组可删」；改为 fail-closed 整体报错
    let rows = store
        .with_conn(|c| {
            let mut stmt = c.prepare("SELECT group_id, uid FROM group_members")?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .map_err(|e| {
            crate::fs_utils::app_log(
                &state.data_dir,
                &format!("读取 group_members 失败，共享分组列表已 fail-closed: {e}"),
            );
            format!("共享分组列表读取失败（group_members）: {e}")
        })?;
    for (gid, uid) in rows {
        uids_by_gid.entry(gid).or_default().push(uid);
    }
    // Buddy / Qoder：池账号 group_id 字段（行文档 data JSON），各一次全量。
    // 同 Trae 臂：`rows_all` 失败不再 `unwrap_or_default` 当空表——与
    // delete_inner（leftover_count>0 整批 fail-closed）、move/assign
    // （计数不符整体回滚）保持同一 fail-closed 口径
    for table in ["wb_accounts", "qoder_accounts"] {
        let rows = store.rows_all(table).map_err(|e| {
            crate::fs_utils::app_log(
                &state.data_dir,
                &format!("读取 {table} 失败，共享分组列表已 fail-closed: {e}"),
            );
            format!("共享分组列表读取失败（{table}）: {e}")
        })?;
        for (_pk, data) in rows {
            let (Some(gid), Some(id)) = (
                data.get("group_id").and_then(serde_json::Value::as_str),
                data.get("id").and_then(serde_json::Value::as_str),
            ) else {
                continue;
            };
            if !gid.is_empty() {
                uids_by_gid.entry(gid.to_string()).or_default().push(id.to_string());
            }
        }
    }
    Ok(defs
        .into_iter()
        .map(|g| {
            let uids = uids_by_gid.remove(&g.id).unwrap_or_default();
            let count = uids.len();
            GroupView { id: g.id, name: g.name, color: g.color, order: g.order, count, uids }
        })
        .collect())
}

/// 各池视图组装（三池既有 list 命令共用）：defs 读统一存储，uids 仍从各自池推导。
/// uids_by_gid 由调用方**一次**整池扫描构建（防 N+1：禁止在每分组闭包内重载池）。
pub(crate) fn list_for_pool(
    state: &AppState,
    uids_by_gid: std::collections::HashMap<String, Vec<String>>,
) -> Vec<GroupView> {
    defs_load(state)
        .into_iter()
        .map(|g| {
            let uids = uids_by_gid.get(&g.id).cloned().unwrap_or_default();
            let count = uids.len();
            GroupView { id: g.id, name: g.name, color: g.color, order: g.order, count, uids }
        })
        .collect()
}

// ── 增删改 ──────────────────────────────────────────────────────────────────

/// 创建共享分组（shg_ 前缀 + 全局重名校验）。
/// 持 group_defs_lock：defs「load→save」整体互斥，防并发创建/删除互相覆盖
pub(crate) fn create_inner(state: &AppState, name: String, color: String) -> Result<String, String> {
    let _guard = state.group_defs_lock.lock().unwrap_or_else(|e| e.into_inner());
    let mut defs = defs_load(state);
    let name = validate_name(&defs, &name, None)?;
    let rand = uuid::Uuid::new_v4().simple().to_string();
    let id = format!("shg_{}{}", chrono::Local::now().timestamp_millis(), &rand[..4]);
    let order = (defs.len() as i32) + 1;
    defs.push(Group { id: id.clone(), name, color, order });
    defs_save(state, &defs)?;
    Ok(id)
}

pub fn shared_groups_create(state: &AppState, name: String, color: String) -> Result<String, String> {
    create_inner(state, name, color)
}

/// 持 group_defs_lock：defs「load→save」整体互斥（同 create_inner）
pub(crate) fn update_inner(
    state: &AppState,
    id: String,
    name: Option<String>,
    color: Option<String>,
    order: Option<i32>,
) -> Result<(), String> {
    let _guard = state.group_defs_lock.lock().unwrap_or_else(|e| e.into_inner());
    let mut defs = defs_load(state);
    // 先校验后可变借用（规避 iter_mut 与校验读借用冲突，对齐 qoder 实现）
    let new_name = match name.as_deref() {
        Some(n) => Some(validate_name(&defs, n, Some(&id))?),
        None => None,
    };
    let g = defs.iter_mut().find(|g| g.id == id).ok_or("分组不存在")?;
    if let Some(n) = new_name {
        g.name = n;
    }
    if let Some(c) = color {
        g.color = c;
    }
    if let Some(o) = order {
        g.order = o;
    }
    defs_save(state, &defs)
}

pub fn shared_groups_update(
    state: &AppState,
    id: String,
    name: Option<String>,
    color: Option<String>,
    order: Option<i32>,
) -> Result<(), String> {
    update_inner(state, id, name, color, order)
}

/// 删除共享分组（顺序敏感，见模块注释）：
/// ①a Trae 成员回落 → ①b Buddy 成员回落 + ② 删定义（同一临界区，防幽灵
/// TOCTOU，见段内注释）→ ①c Qoder 成员回落（定义已删，并发 move 复检必拒）
/// → ③ 清 api_pool / api_keys.allowed_groups 引用 → ④ 热重载 + org 索引失效。
pub(crate) fn delete_inner(state: &AppState, id: &str) -> Result<(), String> {
    let store = crate::store::db(&state.data_dir);
    // 预检（审查 B5）：池表读失败 fail-closed 中止删除——防止误当空池跳过成员
    // 回落，删定义后留下悬空 group_id 引用；此处零副作用，报错后可重试
    crate::store::docs::wb_pool_load_raw(&store)?;
    crate::store::docs::qoder_pool_load_raw(&store)?;
    // ①a Trae 成员回落：直接删 group_members 行（缺失 = 未分组，幂等）
    store.with_conn(|c| {
        c.execute("DELETE FROM group_members WHERE group_id = ?1", [id])?;
        Ok(())
    })?;
    // ①b Buddy 成员回落（读-改-写互斥：罩住池段防旧副本整文档覆盖；读改用
    // fail-closed 版本——中途读失败即中止，防半回落状态）
    // ② 删定义并入本临界区（持 wb_pool_lock 期间取 group_defs_lock，锁序池锁→defs锁）：
    // 关闭「回落释放池锁后、删定义前」的幽灵 TOCTOU 窗口——并发 workbuddy_account_move
    // 在该窗口持池锁+defs锁复检会通过（定义还在）并把账号绑进即将删除的分组。
    // 删定义后 ①c 的 Qoder 回落仍照常执行：此刻后并发的 move 复检必失败（定义已删），
    // 先于删定义完成的 move 绑定会被 ①c 回落清除——两侧窗口均闭合
    {
        let _guard = state.wb_pool_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut pool = crate::store::docs::wb_pool_load_raw(&store)?;
        let mut changed = false;
        if let Some(arr) = pool.get_mut("accounts").and_then(serde_json::Value::as_array_mut) {
            for a in arr.iter_mut() {
                if a.get("group_id").and_then(|v| v.as_str()) == Some(id) {
                    a["group_id"] = serde_json::json!("");
                    changed = true;
                }
            }
        }
        if changed {
            crate::store::docs::wb_pool_save(&store, &pool)?;
        }
        // ② 删定义（同一临界区内取 defs 锁）
        {
            let _defs = state.group_defs_lock.lock().unwrap_or_else(|e| e.into_inner());
            // ①a 复删 Trae 成员（幂等）：闭合「①a 已删、② 未删定义」窗口内并发
            // group_move 持 defs 锁复检通过、把账号重新绑进本分组的幽灵绑定
            // （删定义后 membership 悬空）。复检与写入在 defs 锁内整体串行，
            // 此处的 DELETE 要么先于并发写入完成（随后被清除），要么并发写入
            // 因定义已删被复检拒绝——两侧窗口均闭合
            store.with_conn(|c| {
                c.execute("DELETE FROM group_members WHERE group_id = ?1", [id])?;
                Ok(())
            })?;
            let mut defs = defs_load(state);
            defs.retain(|g| g.id != id);
            defs_save(state, &defs)?;
        }
    }
    // ①c Qoder 成员回落（Value 语义直改保留未知字段，持 qoder_pool_lock——
    // qoder::common 为私有模块，此处与 WB 侧同构实现；定义已删，此刻后并发的
    // qoder_account_move 复检必拒）
    {
        let _guard = state.qoder_pool_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut pool = crate::store::docs::qoder_pool_load_raw(&store)?;
        let mut changed = false;
        if let Some(arr) = pool.get_mut("accounts").and_then(serde_json::Value::as_array_mut) {
            for a in arr.iter_mut() {
                if a.get("group_id").and_then(|v| v.as_str()) == Some(id) {
                    a["group_id"] = serde_json::json!("");
                    changed = true;
                }
            }
        }
        if changed {
            crate::store::docs::qoder_pool_save(&store, &pool)?;
        }
    }
    // ③ 清引用：api_pool 三池分组筛选 + api_keys.allowed_groups
    // 读改写基线统一走迁移入口（对齐上游 P1 审查修复）：直读 kv_get 会落 serde
    // default，整表写回把 Buddy 池旧共享迁移值静默覆盖丢失；runtime.rs::load_pool_file
    // 即为此统一入口（pool_set / 各池 groups 写回共用）。
    // 读-改-写互斥（审查 B1）：整文档覆盖写，持 api_pool_cfg_lock 防并发写点互覆
    // （此时 ①② 段池锁/org 锁均已释放，锁序「池/组织 → 本」单向成立）
    let mut refs_changed = false;
    {
        let _pool_cfg = crate::api_server::runtime::api_pool_cfg_lock();
        let mut pool_file = crate::api_server::runtime::load_pool_file(&state.data_dir);
        for ids in [
            &mut pool_file.group_ids,
            &mut pool_file.wb_group_ids,
            &mut pool_file.qoder_group_ids,
        ] {
            if ids.iter().any(|g| g == id) {
                ids.retain(|g| g != id);
                refs_changed = true;
            }
        }
        if refs_changed {
            store.kv_set("api_pool", &pool_file)?;
        }
    }
    // Key 引用清理走锁内原子修改（防与并发 UI 保存互覆约束字段）。
    // 审查修复（中等项，移植 2026-10-13）：modify 返回值（是否清理了引用）不丢弃，留痕可观测
    let keys_cleaned = crate::api_server::api_keys::modify(&state.data_dir, |keys_file| {
        let mut changed = false;
        for k in keys_file.keys.iter_mut() {
            if k.allowed_groups.iter().any(|g| g == id) {
                k.allowed_groups.retain(|g| g != id);
                changed = true;
            }
        }
        changed
    });
    if keys_cleaned {
        crate::fs_utils::app_log(
            &state.data_dir,
            &format!("分组 {id} 已删除：同步清理了引用它的 API Key allowed_groups"),
        );
    }
    // ④ 联动：成员回落 / 池筛选变更都影响调度池；Key 约束展开索引失效
    if refs_changed {
        crate::api_server::runtime::reload_pools_after_change(state);
    }
    crate::api_server::org_index::invalidate(&state.data_dir);
    Ok(())
}

pub fn shared_groups_delete(state: &AppState, id: String) -> Result<(), String> {
    delete_inner(state, &id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Group;

    fn tmp_state(tag: &str) -> (std::path::PathBuf, AppState) {
        let d = std::env::temp_dir().join(format!("twa_shg_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let state = AppState {
            data_dir: d.clone(),
            jwt_refresh_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
            qoder_pool_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
            wb_pool_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
            buddy_ai_pool_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
            group_defs_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
            events: std::sync::Arc::new(std::sync::Mutex::new(None)),
        };
        (d, state)
    }

    #[test]
    fn validate_name_global_unique() {
        let defs = vec![Group { id: "shg_1".into(), name: "主号".into(), color: String::new(), order: 1 }];
        assert!(validate_name(&defs, "  主号 ", None).is_err(), "全局重名拒绝");
        assert_eq!(validate_name(&defs, "  主号 ", Some("shg_1")).unwrap(), "主号", "排除自身后放行");
        assert!(validate_name(&defs, "   ", None).is_err(), "空名拒绝");
    }

    #[test]
    fn create_update_delete_roundtrip() {
        let (dir, state) = tmp_state("crud");
        let id = create_inner(&state, "主号".into(), "#f00".into()).unwrap();
        assert!(id.starts_with("shg_"));
        // 重名拒绝（全局）
        assert!(create_inner(&state, "主号".into(), "#0f0".into()).is_err());
        update_inner(&state, id.clone(), Some("主力".into()), None, None).unwrap();
        let defs = defs_load(&state);
        assert_eq!(defs[0].name, "主力");
        // 删除：无成员无引用也应成功（幂等空操作）
        delete_inner(&state, &id).unwrap();
        assert!(defs_load(&state).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
