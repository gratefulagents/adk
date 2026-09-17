use crate::{
    storage::{Backend, Transaction, normalize_options},
    *,
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

pub trait StateHooks: Send + Sync {
    fn prepare_memory(&self, _input: &mut UpsertMemoryInput) -> Result<()> {
        Ok(())
    }
    fn before_write(&self, _event: &Event) -> Result<()> {
        Ok(())
    }
    fn after_write(&self, _event: &Event) {}
    fn before_embed(&self, _texts: &[String]) -> Result<()> {
        Ok(())
    }
}
struct NoHooks;
impl StateHooks for NoHooks {}

pub struct ProjectStore {
    pub(crate) backend: Backend,
    pub(crate) options: StoreOptions,
    pub(crate) hooks: Arc<dyn StateHooks>,
    state_dir: PathBuf,
}
#[derive(Debug, Clone, Default)]
pub struct State {
    pub project: Project,
    pub tasks: BTreeMap<String, Task>,
    pub memories: BTreeMap<String, Memory>,
    pub sessions: BTreeMap<String, SessionSummary>,
    pub last_seq: i64,
}
impl State {
    pub fn replay(events: &[Event]) -> Result<Self> {
        let mut state = Self::default();
        for event in events {
            state.apply(event)?;
            state.last_seq = state.last_seq.max(event.seq);
        }
        state.recompute_blocks();
        Ok(state)
    }
    fn apply(&mut self, ev: &Event) -> Result<()> {
        let p = &ev.payload;
        let id = p["id"].as_str().unwrap_or_default();
        let at = || -> Result<DateTime<Utc>> { Ok(serde_json::from_value(p["at"].clone())?) };
        match ev.event_type.as_str() {
            "project.initialized" => {
                self.project = serde_json::from_value(p.clone())?;
                if self.project.schema_version != SCHEMA_VERSION {
                    return Err(Error::Invalid("unsupported project schema version".into()));
                }
            }
            "task.created" => {
                let task: Task = serde_json::from_value(p.clone())?;
                self.tasks.insert(task.id.clone(), task);
            }
            "task.updated" => {
                if p["task"]["id"].as_str().is_some_and(|id| !id.is_empty()) {
                    let task: Task = serde_json::from_value(p["task"].clone())?;
                    self.tasks.insert(task.id.clone(), task);
                }
            }
            "task.claimed" => {
                let task = self.task_mut(id)?;
                task.assignee = p["actor"].as_str().unwrap_or_default().into();
                task.status = "in_progress".into();
                task.updated_at = at()?;
                task.closed_at = None;
            }
            "task.closed" => {
                if p["task"]["id"].as_str().is_some_and(|s| !s.is_empty()) {
                    let task: Task = serde_json::from_value(p["task"].clone())?;
                    self.tasks.insert(task.id.clone(), task);
                } else {
                    let task = self.task_mut(id)?;
                    task.status = "closed".into();
                    task.updated_at = at()?;
                    task.closed_at = Some(at()?);
                    if let Some(reason) = p["reason"].as_str().filter(|r| !r.is_empty()) {
                        task.comments.push(TaskComment {
                            id: format!("comment_{}", ev.event_id),
                            actor: ev.actor.clone(),
                            body: format!("Closed: {reason}"),
                            created_at: at()?,
                        });
                    }
                }
            }
            "task.comment_added" => {
                let comment: TaskComment = serde_json::from_value(p["comment"].clone())?;
                let task = self.task_mut(id)?;
                task.updated_at = comment.created_at;
                task.comments.push(comment);
            }
            "task.dependency_added" | "task.dependency_removed" => {
                let dep = p["depends_on"].as_str().unwrap_or_default();
                let task = self.task_mut(id)?;
                if ev.event_type == "task.dependency_added" {
                    task.depends_on.push(dep.into());
                    task.depends_on = unique(&task.depends_on);
                } else {
                    task.depends_on.retain(|s| s != dep);
                }
                task.updated_at = at()?;
            }
            "memory.upserted" => {
                let memory: Memory = serde_json::from_value(p.clone())?;
                self.memories.insert(memory.id.clone(), memory);
            }
            "memory.deleted" => {
                self.memories.remove(id);
            }
            "session.summary_saved" => {
                let s: SessionSummary = serde_json::from_value(p.clone())?;
                self.sessions.insert(s.id.clone(), s);
            }
            _ => {}
        }
        Ok(())
    }
    fn task_mut(&mut self, id: &str) -> Result<&mut Task> {
        self.tasks
            .get_mut(id)
            .ok_or_else(|| Error::NotFound(format!("task {id:?}")))
    }
    fn recompute_blocks(&mut self) {
        let links: Vec<_> = self
            .tasks
            .values()
            .flat_map(|t| t.depends_on.iter().map(|d| (t.id.clone(), d.clone())))
            .collect();
        for t in self.tasks.values_mut() {
            t.blocks.clear();
        }
        for (id, dep) in links {
            if let Some(t) = self.tasks.get_mut(&dep) {
                if !t.blocks.contains(&id) {
                    t.blocks.push(id);
                }
            }
        }
    }
    pub fn sorted_tasks(&self) -> Vec<Task> {
        let mut v: Vec<_> = self.tasks.values().cloned().collect();
        v.sort_by(|a, b| {
            a.status
                .cmp(&b.status)
                .then(a.priority.cmp(&b.priority))
                .then(b.updated_at.cmp(&a.updated_at))
                .then(a.id.cmp(&b.id))
        });
        v
    }
    pub fn sorted_memories(&self) -> Vec<Memory> {
        let mut v: Vec<_> = self.memories.values().cloned().collect();
        v.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id)));
        v
    }
    pub fn sorted_sessions(&self) -> Vec<SessionSummary> {
        let mut v: Vec<_> = self.sessions.values().cloned().collect();
        v.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id)));
        v
    }
    fn has_blocker(&self, task: &Task) -> bool {
        task.depends_on
            .iter()
            .any(|id| self.tasks.get(id).is_none_or(|t| t.status != "closed"))
    }
    fn ready(&self, f: &TaskFilter) -> Vec<Task> {
        let actor = first(&f.actor, &f.assignee);
        let mut tasks: Vec<_> = self
            .sorted_tasks()
            .into_iter()
            .filter(|t| {
                t.status == "open"
                    && matches_labels(&t.labels, &f.labels)
                    && (f.assignee.is_empty() || t.assignee.is_empty() || t.assignee == f.assignee)
                    && (f.include_assigned || t.assignee.is_empty() || t.assignee == actor)
                    && !self.has_blocker(t)
            })
            .collect();
        limit(&mut tasks, f.limit);
        tasks
    }
}
impl ProjectStore {
    pub fn filesystem(options: FilesystemOptions) -> Result<Self> {
        let store = normalize_options(options.store)?;
        let path = if options.state_dir.as_os_str().is_empty() {
            default_state_dir(&store.project_id)?
        } else {
            std::path::absolute(options.state_dir)?
        };
        let backend = Backend::filesystem(path.clone(), options.lock_timeout)?;
        Self::open(backend, store, path)
    }
    pub fn sqlite(options: SQLiteOptions) -> Result<Self> {
        let store = normalize_options(options.store)?;
        if options.path.as_os_str().is_empty() {
            return Err(Error::Invalid("SQLite path is required".into()));
        }
        let path = std::path::absolute(options.path)?;
        let backend = Backend::sqlite(&path, &options.table_prefix, &store.project_id)?;
        Self::open(backend, store, path.parent().unwrap().into())
    }
    pub fn sqlite_connection(
        connection: rusqlite::Connection,
        table_prefix: &str,
        options: StoreOptions,
    ) -> Result<Self> {
        let options = normalize_options(options)?;
        let backend = Backend::connection(connection, table_prefix, &options.project_id)?;
        Self::open(backend, options, PathBuf::new())
    }
    fn open(backend: Backend, options: StoreOptions, state_dir: PathBuf) -> Result<Self> {
        let store = Self {
            backend,
            options,
            state_dir,
            hooks: Arc::new(NoHooks),
        };
        store.backend.transaction(|tx| {
            let mut state = State::replay(&tx.events()?)?;
            if state.last_seq == 0 {
                let now = Utc::now();
                let project = Project {
                    schema_version: SCHEMA_VERSION,
                    project_id: store.options.project_id.clone(),
                    workdir: store.options.work_dir.to_string_lossy().into(),
                    state_dir: store.state_dir.to_string_lossy().into(),
                    created_at: now,
                    updated_at: now,
                };
                let event = store.event(
                    1,
                    "project.initialized",
                    serde_json::to_value(project)?,
                    now,
                );
                tx.append(&event)?;
                state.apply(&event)?;
                state.last_seq = 1;
            } else if state.project.project_id != store.options.project_id {
                return Err(Error::Invalid("state belongs to another project".into()));
            }
            let orphans: Vec<_> = tx
                .embeddings()?
                .keys()
                .filter(|id| !state.memories.contains_key(*id))
                .cloned()
                .collect();
            if !orphans.is_empty() {
                tx.delete_embeddings(&orphans)?;
            }
            tx.snapshot(&state)
        })?;
        Ok(store)
    }
    pub fn with_hooks(mut self, hooks: Arc<dyn StateHooks>) -> Self {
        self.hooks = hooks;
        self
    }
    pub fn project_id(&self) -> &str {
        &self.options.project_id
    }
    pub fn state_dir(&self) -> &std::path::Path {
        &self.state_dir
    }
    pub fn events(&self) -> Result<Vec<Event>> {
        self.backend.transaction(|tx| tx.events())
    }
    pub fn state(&self) -> Result<State> {
        State::replay(&self.events()?)
    }
    fn event(&self, seq: i64, kind: &str, payload: Value, time: DateTime<Utc>) -> Event {
        Event {
            seq,
            event_id: id("evt"),
            project_id: self.options.project_id.clone(),
            run_id: self.options.run_id.clone(),
            actor: self.options.actor.clone(),
            time,
            event_type: kind.into(),
            payload,
        }
    }
    fn mutate<T>(
        &self,
        kind: &str,
        f: impl FnOnce(&mut State, DateTime<Utc>) -> Result<(Value, T)>,
    ) -> Result<T> {
        let (out, event) = self.backend.transaction(|tx| {
            let mut state = State::replay(&tx.events()?)?;
            let now = Utc::now();
            let (payload, out) = f(&mut state, now)?;
            let event = self.event(state.last_seq + 1, kind, payload, now);
            self.hooks.before_write(&event)?;
            state.apply(&event)?;
            state.recompute_blocks();
            state.project.updated_at = now;
            if kind == "memory.deleted" {
                tx.delete_embeddings(&[event.payload["id"].as_str().unwrap().into()])?;
            }
            tx.append(&event)?;
            tx.snapshot(&state)?;
            Ok((out, event))
        })?;
        self.hooks.after_write(&event);
        Ok(out)
    }
    pub fn create_task(&self, input: CreateTaskInput) -> Result<Task> {
        self.mutate("task.created", |_, now| {
            let title = required(&input.title, "title")?;
            let task = Task {
                id: id("task"),
                title,
                description: input.description.trim().into(),
                task_type: normalize_type(&input.task_type),
                status: "open".into(),
                priority: input.priority.clamp(0, 4),
                assignee: input.assignee.trim().into(),
                depends_on: unique(&input.depends_on),
                labels: unique(&input.labels),
                created_at: now,
                updated_at: now,
                source_run: first(&input.source_run, &self.options.run_id),
                metadata: input.metadata,
                ..Default::default()
            };
            Ok((serde_json::to_value(&task)?, task))
        })
    }
    pub fn update_task(&self, id: &str, patch: TaskPatch) -> Result<Task> {
        self.mutate("task.updated", |state, now| {
            let mut task = state.task_mut(id.trim())?.clone();
            if let Some(v) = &patch.title {
                task.title = v.trim().into();
            }
            if let Some(v) = &patch.description {
                task.description = v.trim().into();
            }
            if let Some(v) = &patch.task_type {
                task.task_type = normalize_type(v);
            }
            if let Some(v) = &patch.status {
                task.status = normalize_status(v);
                task.closed_at = (task.status == "closed").then_some(now);
            }
            if let Some(v) = patch.priority {
                task.priority = v.clamp(0, 4);
            }
            if let Some(v) = &patch.assignee {
                task.assignee = v.trim().into();
            }
            if patch.replace_labels {
                task.labels = unique(&patch.labels);
            }
            if let Some(v) = &patch.metadata {
                task.metadata = v.clone();
            }
            task.updated_at = now;
            Ok((json!({"id":task.id,"patch":patch,"task":task}), task))
        })
    }
    pub fn claim_task(&self, id: &str, actor: &str) -> Result<Task> {
        self.mutate("task.claimed", |state, now| {
            let mut task = state.task_mut(id.trim())?.clone();
            let actor = first(&first(actor, &self.options.actor), "agent");
            task.assignee = actor.clone();
            task.status = "in_progress".into();
            task.updated_at = now;
            task.closed_at = None;
            Ok((json!({"id":task.id,"actor":actor,"at":now}), task))
        })
    }
    pub fn close_task(&self, id: &str, reason: &str) -> Result<Task> {
        self.mutate("task.closed", |state, now| {
            let mut task = state.task_mut(id.trim())?.clone();
            task.status = "closed".into();
            task.updated_at = now;
            task.closed_at = Some(now);
            if !reason.trim().is_empty() {
                task.comments.push(TaskComment {
                    id: crate::id("comment"),
                    actor: self.options.actor.clone(),
                    body: format!("Closed: {}", reason.trim()),
                    created_at: now,
                });
            }
            Ok((
                json!({"id":task.id,"reason":reason.trim(),"at":now,"task":task}),
                task,
            ))
        })
    }
    pub fn ready_tasks(&self, filter: TaskFilter) -> Result<Vec<Task>> {
        Ok(self.state()?.ready(&filter))
    }
    pub fn list_tasks(&self) -> Result<Vec<Task>> {
        Ok(self.state()?.sorted_tasks())
    }
    pub fn get_task(&self, id: &str) -> Result<Task> {
        self.state()?
            .tasks
            .remove(id.trim())
            .ok_or_else(|| Error::NotFound(format!("task {id:?}")))
    }
    pub fn add_dependency(&self, task_id: &str, depends_on: &str) -> Result<()> {
        self.dependency(task_id, depends_on, true)
    }
    pub fn remove_dependency(&self, task_id: &str, depends_on: &str) -> Result<()> {
        self.dependency(task_id, depends_on, false)
    }
    fn dependency(&self, task_id: &str, depends_on: &str, add: bool) -> Result<()> {
        self.mutate(
            if add {
                "task.dependency_added"
            } else {
                "task.dependency_removed"
            },
            |state, now| {
                let task_id = task_id.trim();
                let depends_on = depends_on.trim();
                state.task_mut(task_id)?;
                if add {
                    if !state.tasks.contains_key(depends_on) {
                        return Err(Error::NotFound(format!("dependency task {depends_on:?}")));
                    }
                    if task_id == depends_on {
                        return Err(Error::Invalid("task cannot depend on itself".into()));
                    }
                }
                Ok((json!({"id":task_id,"depends_on":depends_on,"at":now}), ()))
            },
        )
    }
    pub fn add_comment(&self, task_id: &str, actor: &str, body: &str) -> Result<TaskComment> {
        self.mutate("task.comment_added", |state, now| {
            let task = state.task_mut(task_id.trim())?;
            let comment = TaskComment {
                id: id("comment"),
                actor: first(actor, &self.options.actor),
                body: required(body, "comment body")?,
                created_at: now,
            };
            Ok((json!({"id":task.id,"comment":comment}), comment))
        })
    }
    pub fn upsert_memory(&self, mut input: UpsertMemoryInput) -> Result<Memory> {
        self.hooks.prepare_memory(&mut input)?;
        self.mutate("memory.upserted", |state, now| {
            let id = if input.id.trim().is_empty() {
                id("mem")
            } else {
                input.id.trim().into()
            };
            let memory = Memory {
                created_at: state.memories.get(&id).map(|m| m.created_at).unwrap_or(now),
                id,
                kind: normalize(
                    &input.kind,
                    &["pinned", "semantic", "episodic", "procedural"],
                    "semantic",
                ),
                scope: normalize(
                    &input.scope,
                    &["project", "user", "task", "file"],
                    "project",
                ),
                content: required(&input.content, "memory content")?,
                tags: unique(&input.tags),
                task_ids: unique(&input.task_ids),
                file_paths: unique(&input.file_paths),
                source_run: first(&input.source_run, &self.options.run_id),
                updated_at: now,
                metadata: input.metadata,
                last_read_at: None,
            };
            Ok((serde_json::to_value(&memory)?, memory))
        })
    }
    pub fn list_memories(&self, filter: MemoryFilter) -> Result<Vec<Memory>> {
        Ok(filter_memories(self.state()?.sorted_memories(), &filter))
    }
    pub fn search_memories(&self, filter: MemoryFilter) -> Result<Vec<Memory>> {
        required(&filter.query, "query")?;
        self.list_memories(filter)
    }
    pub fn delete_memory(&self, id: &str) -> Result<()> {
        self.mutate("memory.deleted", |state, now| {
            let id = id.trim();
            if !state.memories.contains_key(id) {
                return Err(Error::NotFound(format!("memory {id:?}")));
            }
            Ok((json!({"id":id,"at":now}), ()))
        })
    }
    pub fn save_session_summary(&self, mut summary: SessionSummary) -> Result<SessionSummary> {
        self.mutate("session.summary_saved", |state, now| {
            required(&summary.summary, "session summary")?;
            if summary.id.trim().is_empty() {
                summary.id = id("session");
                summary.created_at = now;
            }
            if let Some(old) = state.sessions.get(&summary.id) {
                summary.created_at = old.created_at;
            }
            summary.run_id = first(&summary.run_id, &self.options.run_id);
            summary.task_ids = unique(&summary.task_ids);
            summary.updated_at = now;
            Ok((serde_json::to_value(&summary)?, summary))
        })
    }
    pub fn list_session_summaries(&self, count: i32) -> Result<Vec<SessionSummary>> {
        let mut v = self.state()?.sorted_sessions();
        limit(&mut v, count);
        Ok(v)
    }
    pub fn memory_stats(&self, filter: MemoryFilter) -> Result<MemoryStats> {
        let memories = self.list_memories(filter)?;
        let mut stats = MemoryStats {
            total: memories.len(),
            ..Default::default()
        };
        for m in memories {
            *stats.by_kind.entry(m.kind).or_default() += 1;
            *stats.by_scope.entry(m.scope).or_default() += 1;
            for tag in m.tags {
                *stats.by_tag.entry(tag).or_default() += 1;
            }
        }
        Ok(stats)
    }
    pub fn prime_context(&self, mut opts: PrimeOptions) -> Result<String> {
        let state = self.state()?;
        opts.actor = first(&opts.actor, &self.options.actor);
        if opts.ready_limit <= 0 {
            opts.ready_limit = 8;
        }
        if opts.memory_limit <= 0 {
            opts.memory_limit = 8;
        }
        let mut out = format!(
            "## Durable Project State\nProject: {}\n",
            state.project.project_id
        );
        if !state.project.workdir.is_empty() {
            out += &format!("Workspace: {}\n", state.project.workdir);
        }
        if let Some(task) = state.tasks.get(&opts.active_task_id).or_else(|| {
            state.tasks.values().find(|t| {
                t.status == "in_progress" && (opts.actor.is_empty() || t.assignee == opts.actor)
            })
        }) {
            out += "\n### Active Task\n";
            out += &task_line(task);
            if !task.description.is_empty() {
                out += &format!("  {}\n", one_line(&task.description, 220));
            }
            if !task.depends_on.is_empty() {
                out += &format!("  Depends on: {}\n", task.depends_on.join(", "));
            }
        }
        let ready = state.ready(&TaskFilter {
            actor: opts.actor,
            limit: opts.ready_limit,
            ..Default::default()
        });
        if !ready.is_empty() {
            out += "\n### Ready Work\n";
            for t in ready {
                out += &task_line(&t);
            }
        }
        let blocked: Vec<_> = state
            .sorted_tasks()
            .into_iter()
            .filter(|t| (t.status == "open" || t.status == "blocked") && state.has_blocker(t))
            .take(5)
            .collect();
        if !blocked.is_empty() {
            out += "\n### Blocked Work\n";
            for t in blocked {
                out += &task_line(&t);
            }
        }
        let memories = filter_memories(
            state.sorted_memories(),
            &MemoryFilter {
                limit: opts.memory_limit,
                ..Default::default()
            },
        );
        for (pinned, heading, max) in [
            (true, "Pinned Memories", 220),
            (false, "Recent Memories", 180),
        ] {
            let entries: Vec<_> = memories
                .iter()
                .filter(|m| (m.kind == "pinned") == pinned)
                .collect();
            if !entries.is_empty() {
                out += &format!("\n### {heading}\n");
                for m in entries {
                    let suffix = if m.tags.is_empty() {
                        m.kind.clone()
                    } else {
                        format!("{}; {}", m.kind, m.tags.join(","))
                    };
                    out += &format!("- {} ({})\n", one_line(&m.content, max), suffix);
                }
            }
        }
        Ok(out.trim().into())
    }
    /// Removes every upsert/deletion event for these IDs, including already-deleted memories.
    /// This is logical history erasure, not guaranteed physical erasure on disk/backups.
    pub fn purge_memories(&self, ids: &[String]) -> Result<usize> {
        let ids = unique(ids);
        let (count, event) = self.backend.transaction(|tx| self.purge_locked(tx, &ids))?;
        self.hooks.after_write(&event);
        Ok(count)
    }
    fn purge_locked(&self, tx: &mut dyn Transaction, ids: &[String]) -> Result<(usize, Event)> {
        let mut events = tx.events()?;
        let notice = self.event(0, "memory.purged", json!({"ids":ids}), Utc::now());
        self.hooks.before_write(&notice)?;
        let old_len = events.len();
        events.retain(|e| {
            !matches!(e.event_type.as_str(), "memory.upserted" | "memory.deleted")
                || !e.payload["id"]
                    .as_str()
                    .is_some_and(|id| ids.iter().any(|i| i == id))
        });
        for (i, e) in events.iter_mut().enumerate() {
            e.seq = i as i64 + 1;
        }
        let state = State::replay(&events)?;
        tx.delete_embeddings(ids)?;
        tx.replace(&events)?;
        tx.snapshot(&state)?;
        Ok((old_len - events.len(), notice))
    }
    pub fn retain_memories(&self, retention: RetentionPolicy) -> Result<Vec<String>> {
        let (ids, notices) = self.backend.transaction(|tx| {
            let mut state = State::replay(&tx.events()?)?;
            let candidates: Vec<_> = state
                .sorted_memories()
                .into_iter()
                .filter(|m| !retention.keep_pinned || m.kind != "pinned")
                .collect();
            let ids: Vec<_> = candidates
                .iter()
                .enumerate()
                .filter(|(i, m)| {
                    retention.before.is_some_and(|before| m.updated_at < before)
                        || retention.max_count.is_some_and(|max| *i >= max)
                })
                .map(|(_, m)| m.id.clone())
                .collect();
            let mut pending = Vec::new();
            if retention.purge_history {
                let (_, notice) = self.purge_locked(tx, &ids)?;
                pending.push(notice);
            } else {
                for (i, id) in ids.iter().enumerate() {
                    let now = Utc::now();
                    let ev = self.event(
                        state.last_seq + i as i64 + 1,
                        "memory.deleted",
                        json!({"id":id,"at":now}),
                        now,
                    );
                    self.hooks.before_write(&ev)?;
                    pending.push(ev);
                }
                tx.delete_embeddings(&ids)?;
                for ev in &pending {
                    state.apply(ev)?;
                    tx.append(ev)?;
                }
                tx.snapshot(&state)?;
            }
            Ok((ids, pending))
        })?;
        for event in notices {
            self.hooks.after_write(&event);
        }
        Ok(ids)
    }
}
#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct MemoryStats {
    pub total: usize,
    pub by_kind: BTreeMap<String, usize>,
    pub by_scope: BTreeMap<String, usize>,
    pub by_tag: BTreeMap<String, usize>,
}
#[derive(Debug, Clone)]
pub struct RetentionPolicy {
    pub before: Option<DateTime<Utc>>,
    pub max_count: Option<usize>,
    pub keep_pinned: bool,
    pub purge_history: bool,
}
impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            before: None,
            max_count: None,
            keep_pinned: true,
            purge_history: false,
        }
    }
}
pub(crate) fn filter_memories(mut memories: Vec<Memory>, f: &MemoryFilter) -> Vec<Memory> {
    let query = f.query.trim().to_lowercase();
    memories.retain(|m| {
        matches_any(&m.kind, &f.kinds)
            && matches_labels(&m.tags, &f.tags)
            && (query.is_empty()
                || haystack(m).contains(&query)
                || query
                    .split_whitespace()
                    .any(|term| haystack(m).contains(term)))
    });
    memories.sort_by(|a, b| {
        (b.kind == "pinned")
            .cmp(&(a.kind == "pinned"))
            .then(b.updated_at.cmp(&a.updated_at))
            .then(a.id.cmp(&b.id))
    });
    limit(&mut memories, f.limit);
    memories
}
pub(crate) fn haystack(m: &Memory) -> String {
    [
        &[m.content.clone(), m.kind.clone(), m.scope.clone()][..],
        &m.tags,
        &m.task_ids,
        &m.file_paths,
    ]
    .concat()
    .join(" ")
    .to_lowercase()
}
pub(crate) fn matches_labels(actual: &[String], wanted: &[String]) -> bool {
    wanted.iter().all(|w| {
        actual
            .iter()
            .any(|a| a.trim().eq_ignore_ascii_case(w.trim()))
    })
}
pub(crate) fn matches_any(actual: &str, wanted: &[String]) -> bool {
    wanted.is_empty()
        || wanted
            .iter()
            .any(|w| actual.trim().eq_ignore_ascii_case(w.trim()))
}
pub(crate) fn required(value: &str, name: &str) -> Result<String> {
    if value.trim().is_empty() {
        Err(Error::Invalid(format!("{name} is required")))
    } else {
        Ok(value.trim().into())
    }
}
pub(crate) fn unique(values: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && seen.insert(s.to_string()))
        .map(str::to_string)
        .collect()
}
fn first(a: &str, b: &str) -> String {
    if a.trim().is_empty() {
        b.trim().into()
    } else {
        a.trim().into()
    }
}
fn normalize(value: &str, valid: &[&str], default: &str) -> String {
    let v = value.trim().to_lowercase();
    if valid.contains(&v.as_str()) {
        v
    } else {
        default.into()
    }
}
fn normalize_type(value: &str) -> String {
    if value.trim().eq_ignore_ascii_case("feat") {
        "feature".into()
    } else {
        normalize(value, &["task", "bug", "feature", "chore", "epic"], "task")
    }
}
fn normalize_status(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "claimed" | "in-progress" => "in_progress".into(),
        "done" | "completed" => "closed".into(),
        _ => normalize(
            value,
            &["open", "in_progress", "blocked", "closed", "deferred"],
            "open",
        ),
    }
}
pub(crate) fn limit<T>(values: &mut Vec<T>, count: i32) {
    if count > 0 {
        values.truncate(count as usize);
    }
}
fn one_line(s: &str, max: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.len() <= max {
        s
    } else {
        let mut cut = max - 3;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}...", &s[..cut])
    }
}
fn task_line(t: &Task) -> String {
    format!(
        "- {} [P{} {}] {}\n",
        t.id,
        t.priority,
        t.status,
        one_line(&t.title, 180)
    )
}
