use std::path::Path;

use crate::models::{
    ActionKind, ActionRequest, ApprovalGrant, PolicyDecision, PolicyVerdict, ScopeDelta,
    SessionPolicy,
};

pub fn evaluate_request(
    policy: &SessionPolicy,
    request: &ActionRequest,
    grants: &[ApprovalGrant],
) -> PolicyDecision {
    if grants.iter().any(|grant| grant.request_id == request.id) {
        return PolicyDecision {
            verdict: PolicyVerdict::Allow,
            reason: "Existing approval grant matched request.".into(),
            requires_approval: false,
            scope_delta: None,
        };
    }

    match request.kind {
        ActionKind::Command => evaluate_command(policy, request),
        ActionKind::File => evaluate_file(policy, request),
        ActionKind::Network => evaluate_network(policy, request),
        ActionKind::App => evaluate_app(policy, request),
        ActionKind::Mcp => evaluate_mcp(policy, request),
    }
}

fn evaluate_command(policy: &SessionPolicy, request: &ActionRequest) -> PolicyDecision {
    let command = request
        .command
        .clone()
        .unwrap_or_default()
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();

    if command.is_empty() {
        return PolicyDecision {
            verdict: PolicyVerdict::Deny,
            reason: "Empty commands are not valid requests.".into(),
            requires_approval: false,
            scope_delta: None,
        };
    }

    if policy
        .allow_commands
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(&command))
    {
        let elevated = policy
            .elevated_commands
            .iter()
            .any(|elevated| elevated.eq_ignore_ascii_case(&command));

        return PolicyDecision {
            verdict: if elevated {
                PolicyVerdict::Prompt
            } else {
                PolicyVerdict::Allow
            },
            reason: if elevated {
                format!("`{command}` is allowed but requires an elevated approval.")
            } else {
                format!("`{command}` is inside the session command allowlist.")
            },
            requires_approval: elevated,
            scope_delta: None,
        };
    }

    PolicyDecision {
        verdict: PolicyVerdict::Prompt,
        reason: format!("`{command}` is outside the allowlist and needs explicit approval."),
        requires_approval: true,
        scope_delta: Some(ScopeDelta {
            add_commands: vec![command],
            ..ScopeDelta::default()
        }),
    }
}

fn evaluate_file(policy: &SessionPolicy, request: &ActionRequest) -> PolicyDecision {
    let target = Path::new(&request.target);
    let in_scope = policy.roots.iter().any(|root| target.starts_with(root));

    if in_scope {
        return PolicyDecision {
            verdict: PolicyVerdict::Allow,
            reason: "Requested path is inside the scoped roots.".into(),
            requires_approval: false,
            scope_delta: None,
        };
    }

    PolicyDecision {
        verdict: PolicyVerdict::Prompt,
        reason: "Requested path is outside the scoped roots.".into(),
        requires_approval: true,
        scope_delta: Some(ScopeDelta {
            add_roots: vec![request.target.clone()],
            ..ScopeDelta::default()
        }),
    }
}

fn evaluate_network(policy: &SessionPolicy, request: &ActionRequest) -> PolicyDecision {
    if policy
        .allow_domains
        .iter()
        .any(|domain| request.target.contains(domain))
    {
        return PolicyDecision {
            verdict: PolicyVerdict::Allow,
            reason: "Destination matched the network allowlist.".into(),
            requires_approval: false,
            scope_delta: None,
        };
    }

    PolicyDecision {
        verdict: PolicyVerdict::Prompt,
        reason: "Destination is outside the network allowlist.".into(),
        requires_approval: true,
        scope_delta: Some(ScopeDelta {
            add_domains: vec![request.target.clone()],
            ..ScopeDelta::default()
        }),
    }
}

fn evaluate_app(policy: &SessionPolicy, request: &ActionRequest) -> PolicyDecision {
    if policy
        .allow_apps
        .iter()
        .any(|app| app.eq_ignore_ascii_case(&request.target))
    {
        return PolicyDecision {
            verdict: PolicyVerdict::Allow,
            reason: "App launch is explicitly allowed.".into(),
            requires_approval: false,
            scope_delta: None,
        };
    }

    PolicyDecision {
        verdict: PolicyVerdict::Prompt,
        reason: "App launch is outside the allowlist.".into(),
        requires_approval: true,
        scope_delta: None,
    }
}

fn evaluate_mcp(policy: &SessionPolicy, request: &ActionRequest) -> PolicyDecision {
    if policy
        .mcp
        .iter()
        .any(|rule| request.target.starts_with(&rule.server))
    {
        return PolicyDecision {
            verdict: PolicyVerdict::Allow,
            reason: "MCP server is permitted by session policy.".into(),
            requires_approval: false,
            scope_delta: None,
        };
    }

    PolicyDecision {
        verdict: PolicyVerdict::Prompt,
        reason: "MCP invocation is not currently allowed.".into(),
        requires_approval: true,
        scope_delta: None,
    }
}
