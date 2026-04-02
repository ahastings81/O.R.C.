use crate::models::{SupervisorTask, Worker, WorkerStatus};

pub fn create_worker(
    name: String,
    adapter: String,
    default_root: String,
    executable_path: Option<String>,
    args: Vec<String>,
) -> Worker {
    Worker {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        adapter,
        status: WorkerStatus::Idle,
        scope_roots: vec![default_root],
        current_task: None,
        executable_path,
        args,
        output_lines: Vec::new(),
    }
}

pub fn assign_task(worker: &mut Worker, title: String, summary: String) -> SupervisorTask {
    let task_id = uuid::Uuid::new_v4().to_string();
    worker.current_task = Some(task_id.clone());
    worker.status = WorkerStatus::Running;

    SupervisorTask {
        id: task_id,
        title,
        assigned_worker_id: Some(worker.id.clone()),
        status: WorkerStatus::Running,
        summary,
    }
}
