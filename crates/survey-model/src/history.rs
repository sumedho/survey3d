//! Undo/redo stack that also produces audit records.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::command::Command;
use crate::document::Document;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditAction {
    Do,
    Undo,
    Redo,
}

impl AuditAction {
    pub fn as_str(self) -> &'static str {
        match self {
            AuditAction::Do => "do",
            AuditAction::Undo => "undo",
            AuditAction::Redo => "redo",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "do" => Some(AuditAction::Do),
            "undo" => Some(AuditAction::Undo),
            "redo" => Some(AuditAction::Redo),
            _ => None,
        }
    }
}

/// One edit, as recorded in the permanent audit trail.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuditRecord {
    /// Milliseconds since the Unix epoch (UTC).
    pub timestamp_ms: i64,
    pub user: String,
    pub action: AuditAction,
    pub label: String,
    pub detail: Value,
}

/// Executes commands and keeps undo/redo stacks plus audit records that have
/// not yet been persisted.
pub struct History {
    /// Commands with their memory size, oldest first.
    undo: Vec<(Box<dyn Command>, usize)>,
    redo: Vec<(Box<dyn Command>, usize)>,
    pending_audit: Vec<AuditRecord>,
    user: String,
    limit: usize,
    /// Most memory the undo steps may hold; the oldest go first beyond it
    /// (the latest step is always kept).
    memory_limit: usize,
    /// Undo steps dropped to stay within the limits.
    dropped: usize,
}

/// Undo memory budget: surfaces and large imports copy a lot into each step.
#[cfg(not(target_arch = "wasm32"))]
pub const UNDO_MEMORY_LIMIT: usize = 512 * 1024 * 1024;
#[cfg(target_arch = "wasm32")]
pub const UNDO_MEMORY_LIMIT: usize = 192 * 1024 * 1024;

impl History {
    pub fn new(user: impl Into<String>) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            pending_audit: Vec::new(),
            user: user.into(),
            limit: 500,
            memory_limit: UNDO_MEMORY_LIMIT,
            dropped: 0,
        }
    }

    pub fn set_user(&mut self, user: impl Into<String>) {
        self.user = user.into();
    }

    pub fn execute(&mut self, mut command: Box<dyn Command>, doc: &mut Document) {
        command.apply(doc);
        self.record(AuditAction::Do, command.as_ref());
        // Sized after applying: that is when it captures what it replaced.
        let size = command.memory_size();
        self.undo.push((command, size));
        self.redo.clear();
        self.trim();
    }

    /// Drops the oldest undo steps beyond the step and memory limits.
    fn trim(&mut self) {
        let mut used: usize = self.undo.iter().map(|(_, s)| s).sum();
        let mut drop = 0;
        while self.undo.len() - drop > 1 && (self.undo.len() - drop > self.limit || used > self.memory_limit)
        {
            used -= self.undo[drop].1;
            drop += 1;
        }
        if drop > 0 {
            self.undo.drain(..drop);
            self.dropped += drop;
        }
    }

    /// Memory the undo and redo steps hold (approximate bytes).
    pub fn memory_used(&self) -> usize {
        self.undo.iter().chain(&self.redo).map(|(_, s)| s).sum()
    }

    /// Undo steps dropped so far to stay within the limits.
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    /// Sets the undo memory budget (bytes).
    pub fn set_memory_limit(&mut self, bytes: usize) {
        self.memory_limit = bytes;
        self.trim();
    }

    pub fn undo(&mut self, doc: &mut Document) -> Option<String> {
        let (mut command, size) = self.undo.pop()?;
        command.revert(doc);
        self.record(AuditAction::Undo, command.as_ref());
        let label = command.label();
        self.redo.push((command, size));
        Some(label)
    }

    pub fn redo(&mut self, doc: &mut Document) -> Option<String> {
        let (mut command, size) = self.redo.pop()?;
        command.apply(doc);
        self.record(AuditAction::Redo, command.as_ref());
        let label = command.label();
        self.undo.push((command, size));
        Some(label)
    }

    pub fn undo_label(&self) -> Option<String> {
        self.undo.last().map(|(c, _)| c.label())
    }

    pub fn redo_label(&self) -> Option<String> {
        self.redo.last().map(|(c, _)| c.label())
    }

    /// Audit records not yet written to storage.
    pub fn pending_audit(&self) -> &[AuditRecord] {
        &self.pending_audit
    }

    /// Removes and returns pending audit records (the store calls this when saving).
    pub fn take_pending_audit(&mut self) -> Vec<AuditRecord> {
        std::mem::take(&mut self.pending_audit)
    }

    /// Puts records back after a failed save so they are not lost.
    pub fn restore_pending_audit(&mut self, mut records: Vec<AuditRecord>) {
        records.append(&mut self.pending_audit);
        self.pending_audit = records;
    }

    /// Clears undo/redo and pending audit (e.g. when opening another project).
    pub fn reset(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.pending_audit.clear();
    }

    fn record(&mut self, action: AuditAction, command: &dyn Command) {
        self.pending_audit.push(AuditRecord {
            timestamp_ms: now_ms(),
            user: self.user.clone(),
            action,
            label: command.label(),
            detail: command.audit_detail(),
        });
    }
}

fn now_ms() -> i64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::ChangeSet;
    use crate::entity::{Entity, SurveyPoint};
    use crate::ids::LayerId;
    use glam::DVec3;

    #[test]
    fn undo_redo_records_audit() {
        let mut doc = Document::new();
        let mut history = History::new("tester");
        let p = Entity::Point(SurveyPoint {
            name: "1".into(),
            position: DVec3::ZERO,
            height_known: true,
            code: "CP".into(),
            description: String::new(),
            layer: LayerId(1),
        });
        history.execute(Box::new(ChangeSet::new("Add point 1").insert_entity(p)), &mut doc);
        assert_eq!(doc.entity_count(), 1);
        assert_eq!(history.undo(&mut doc).as_deref(), Some("Add point 1"));
        assert_eq!(doc.entity_count(), 0);
        history.redo(&mut doc);
        assert_eq!(doc.entity_count(), 1);

        let actions: Vec<_> = history.pending_audit().iter().map(|r| r.action).collect();
        assert_eq!(
            actions,
            vec![AuditAction::Do, AuditAction::Undo, AuditAction::Redo]
        );
        assert!(history.pending_audit().iter().all(|r| r.user == "tester"));
        assert!(history.take_pending_audit().len() == 3 && history.pending_audit().is_empty());
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;
    use crate::{ChangeSet, Surface};
    use glam::DVec3;
    use survey_core::tin::Tin;

    fn surface(n: usize) -> Surface {
        Surface {
            name: "S".into(),
            definition: None,
            tin: Tin {
                vertices: vec![DVec3::ZERO; n],
                triangles: vec![[0, 0, 0]; n * 2],
            },
            style: Default::default(),
        }
    }

    #[test]
    fn oldest_steps_go_when_memory_runs_out() {
        let mut doc = Document::new();
        let mut h = History::new("t");
        // Each rebuild keeps old and new triangles: ~96 bytes per vertex here.
        h.set_memory_limit(10_000_000);
        let mut add = ChangeSet::new("add").insert_surface(surface(10));
        add.apply(&mut doc);
        let id = add.inserted_surface_ids()[0];
        for _ in 0..10 {
            h.execute(
                Box::new(ChangeSet::new("rebuild").update_surface(id, surface(20_000))),
                &mut doc,
            );
        }
        // 20k vertices × (24 + 24 for 2 triangles) × 2 copies ≈ 1.9 MB a step.
        assert!(h.memory_used() <= 10_000_000, "{}", h.memory_used());
        assert!(h.dropped() >= 4, "{}", h.dropped());
        assert!(h.undo_label().is_some());
        // Even a single step over the budget is kept (the latest can always be undone).
        h.set_memory_limit(1);
        assert!(h.dropped() > 4);
        assert!(h.undo(&mut doc).is_some());
        assert!(h.undo(&mut doc).is_none());
    }
}
