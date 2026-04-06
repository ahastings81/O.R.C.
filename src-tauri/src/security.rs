use std::process::Child;

use crate::models::{ProtectionState, ProtectionStatus};

pub struct WorkerOsEnforcement {
    #[cfg(windows)]
    job_handle: isize,
}

impl Drop for WorkerOsEnforcement {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            if self.job_handle != 0 {
                windows_sys::Win32::Foundation::CloseHandle(self.job_handle as _);
            }
        }
    }
}

pub fn detect_host_protections() -> Vec<ProtectionStatus> {
    let mut protections = vec![
        ProtectionStatus {
            id: "sandbox_launch".into(),
            label: "Sandboxed launch".into(),
            state: ProtectionState::Active,
            detail: "Agents start in isolated working directories with stripped environments."
                .into(),
        },
        ProtectionStatus {
            id: "broker_violation_termination".into(),
            label: "Broker violation termination".into(),
            state: ProtectionState::Active,
            detail:
                "Agents are terminated if they emit forbidden approval-style broker envelopes."
                    .into(),
        },
    ];

    #[cfg(windows)]
    {
        protections.push(ProtectionStatus {
            id: "job_object".into(),
            label: "Windows Job Object".into(),
            state: ProtectionState::Active,
            detail:
                "Agents are assigned to a Job Object with kill-on-close lifecycle enforcement."
                    .into(),
        });
        protections.push(ProtectionStatus {
            id: "child_process_block".into(),
            label: "Child process restriction".into(),
            state: ProtectionState::Active,
            detail:
                "Child processes are constrained with a one-process Job Object limit by default."
                    .into(),
        });
        protections.push(ProtectionStatus {
            id: "restricted_token".into(),
            label: "Restricted token".into(),
            state: ProtectionState::Available,
            detail:
                "Reserved for hardened mode on capable Windows systems; baseline launch is active now."
                    .into(),
        });
    }

    #[cfg(not(windows))]
    {
        protections.push(ProtectionStatus {
            id: "job_object".into(),
            label: "Windows Job Object".into(),
            state: ProtectionState::Unsupported,
            detail: "This protection is only available on Windows.".into(),
        });
        protections.push(ProtectionStatus {
            id: "child_process_block".into(),
            label: "Child process restriction".into(),
            state: ProtectionState::Unsupported,
            detail: "This protection is only available on Windows.".into(),
        });
        protections.push(ProtectionStatus {
            id: "restricted_token".into(),
            label: "Restricted token".into(),
            state: ProtectionState::Unsupported,
            detail: "This protection is only available on Windows.".into(),
        });
    }

    protections.push(ProtectionStatus {
        id: "enterprise_policy".into(),
        label: "Enterprise OS policy".into(),
        state: ProtectionState::Optional,
        detail:
            "WDAC, AppLocker, or SRP can add stronger machine-level enforcement when configured."
                .into(),
    });

    protections
}

#[cfg(windows)]
pub fn apply_worker_os_enforcement(child: &Child) -> Result<WorkerOsEnforcement, String> {
    use std::mem::{size_of, zeroed};
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
        JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err("failed to create Windows Job Object".into());
        }

        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
        limits.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        limits.BasicLimitInformation.ActiveProcessLimit = 1;

        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &mut limits as *mut _ as *mut _,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );

        if ok == 0 {
            windows_sys::Win32::Foundation::CloseHandle(job);
            return Err("failed to apply Job Object limits".into());
        }

        let process_handle = child.as_raw_handle() as HANDLE;
        let assigned = AssignProcessToJobObject(job, process_handle);
        if assigned == 0 {
            windows_sys::Win32::Foundation::CloseHandle(job);
            return Err("failed to assign agent process to Job Object".into());
        }

        Ok(WorkerOsEnforcement {
            job_handle: job as isize,
        })
    }
}

#[cfg(not(windows))]
pub fn apply_worker_os_enforcement(_child: &Child) -> Result<WorkerOsEnforcement, String> {
    Ok(WorkerOsEnforcement {})
}
