use crate::models::{
    AgentCapabilityProfile, AgentCapabilitySetting, AgentCompatibility, AgentMemoryMode,
    AgentRuntimeMode, AgentTrustLevel, SupervisorTask, TaskGuardrails, Worker, WorkerStatus,
};

pub fn create_worker(
    name: String,
    adapter: String,
    default_root: String,
    executable_path: Option<String>,
    args: Vec<String>,
    memory_mode: AgentMemoryMode,
) -> Worker {
    Worker {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        adapter,
        trust_level: AgentTrustLevel::Untrusted,
        runtime_mode: AgentRuntimeMode::BrokerOnly,
        compatibility: AgentCompatibility::Unknown,
        capability_profile: AgentCapabilityProfile {
            execution: AgentCapabilitySetting::Brokered,
            filesystem: AgentCapabilitySetting::Scoped,
            network: AgentCapabilitySetting::Prompted,
            memory: AgentCapabilitySetting::Isolated,
            delegation: AgentCapabilitySetting::HumanOnly,
            control_plane: AgentCapabilitySetting::Denied,
        },
        memory_mode,
        profile_id: None,
        status: WorkerStatus::Idle,
        scope_roots: vec![default_root],
        current_task: None,
        executable_path,
        args,
        output_lines: Vec::new(),
    }
}

pub fn assign_task(
    worker: &mut Worker,
    title: String,
    summary: String,
    guardrails: TaskGuardrails,
) -> SupervisorTask {
    let task_id = uuid::Uuid::new_v4().to_string();
    worker.current_task = Some(task_id.clone());
    worker.status = WorkerStatus::Running;

    SupervisorTask {
        id: task_id,
        title,
        assigned_worker_id: Some(worker.id.clone()),
        status: WorkerStatus::Running,
        summary,
        guardrails,
    }
}
