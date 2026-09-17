use crate::*;

pub trait TaskStore: Send + Sync {
    fn create_task(&self, input: CreateTaskInput) -> Result<Task>;
    fn update_task(&self, id: &str, patch: TaskPatch) -> Result<Task>;
    fn claim_task(&self, id: &str, actor: &str) -> Result<Task>;
    fn close_task(&self, id: &str, reason: &str) -> Result<Task>;
    fn ready_tasks(&self, filter: TaskFilter) -> Result<Vec<Task>>;
    fn list_tasks(&self) -> Result<Vec<Task>>;
    fn get_task(&self, id: &str) -> Result<Task>;
    fn add_dependency(&self, task_id: &str, depends_on: &str) -> Result<()>;
    fn remove_dependency(&self, task_id: &str, depends_on: &str) -> Result<()>;
    fn add_comment(&self, task_id: &str, actor: &str, body: &str) -> Result<TaskComment>;
}
impl TaskStore for ProjectStore {
    fn create_task(&self, input: CreateTaskInput) -> Result<Task> {
        ProjectStore::create_task(self, input)
    }
    fn update_task(&self, id: &str, patch: TaskPatch) -> Result<Task> {
        ProjectStore::update_task(self, id, patch)
    }
    fn claim_task(&self, id: &str, actor: &str) -> Result<Task> {
        ProjectStore::claim_task(self, id, actor)
    }
    fn close_task(&self, id: &str, reason: &str) -> Result<Task> {
        ProjectStore::close_task(self, id, reason)
    }
    fn ready_tasks(&self, filter: TaskFilter) -> Result<Vec<Task>> {
        ProjectStore::ready_tasks(self, filter)
    }
    fn list_tasks(&self) -> Result<Vec<Task>> {
        ProjectStore::list_tasks(self)
    }
    fn get_task(&self, id: &str) -> Result<Task> {
        ProjectStore::get_task(self, id)
    }
    fn add_dependency(&self, task_id: &str, depends_on: &str) -> Result<()> {
        ProjectStore::add_dependency(self, task_id, depends_on)
    }
    fn remove_dependency(&self, task_id: &str, depends_on: &str) -> Result<()> {
        ProjectStore::remove_dependency(self, task_id, depends_on)
    }
    fn add_comment(&self, task_id: &str, actor: &str, body: &str) -> Result<TaskComment> {
        ProjectStore::add_comment(self, task_id, actor, body)
    }
}

pub trait MemoryStore: Send + Sync {
    fn upsert_memory(&self, input: UpsertMemoryInput) -> Result<Memory>;
    fn list_memories(&self, filter: MemoryFilter) -> Result<Vec<Memory>>;
    fn search_memories(&self, filter: MemoryFilter) -> Result<Vec<Memory>>;
    fn delete_memory(&self, id: &str) -> Result<()>;
}
impl MemoryStore for ProjectStore {
    fn upsert_memory(&self, input: UpsertMemoryInput) -> Result<Memory> {
        ProjectStore::upsert_memory(self, input)
    }
    fn list_memories(&self, filter: MemoryFilter) -> Result<Vec<Memory>> {
        ProjectStore::list_memories(self, filter)
    }
    fn search_memories(&self, filter: MemoryFilter) -> Result<Vec<Memory>> {
        ProjectStore::search_memories(self, filter)
    }
    fn delete_memory(&self, id: &str) -> Result<()> {
        ProjectStore::delete_memory(self, id)
    }
}

pub trait SessionStore: Send + Sync {
    fn save_session_summary(&self, summary: SessionSummary) -> Result<SessionSummary>;
    fn list_session_summaries(&self, count: i32) -> Result<Vec<SessionSummary>>;
}
impl SessionStore for ProjectStore {
    fn save_session_summary(&self, summary: SessionSummary) -> Result<SessionSummary> {
        ProjectStore::save_session_summary(self, summary)
    }
    fn list_session_summaries(&self, count: i32) -> Result<Vec<SessionSummary>> {
        ProjectStore::list_session_summaries(self, count)
    }
}

pub trait PrimeStore: Send + Sync {
    fn prime_context(&self, opts: PrimeOptions) -> Result<String>;
}
impl PrimeStore for ProjectStore {
    fn prime_context(&self, opts: PrimeOptions) -> Result<String> {
        ProjectStore::prime_context(self, opts)
    }
}

pub trait Store: TaskStore + MemoryStore + SessionStore + PrimeStore {}
impl Store for ProjectStore {}
