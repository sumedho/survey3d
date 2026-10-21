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
    undo: Vec<Box<dyn Command>>,
    redo: Vec<Box<dyn Command>>,
    pending_audit: Vec<AuditRecord>,
    user: String,
    limit: usize,
}

impl History {
    pub fn new(user: impl Into<String>) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            pending_audit: Vec::new(),
            user: user.into(),
            limit: 500,
        }
    }

    pub fn set_user(&mut self, user: impl Into<String>) {
        self.user = user.into();
    }

    pub fn execute(&mut self, mut command: Box<dyn Command>, doc: &mut Document) {
        command.apply(doc);
        self.record(AuditAction::Do, command.as_ref());
        self.undo.push(command);
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn undo(&mut self, doc: &mut Document) -> Option<String> {
        let mut command = self.undo.pop()?;
        command.revert(doc);
        self.record(AuditAction::Undo, command.as_ref());
        let label = command.label();
        self.redo.push(command);
        Some(label)
    }

    pub fn redo(&mut self, doc: &mut Document) -> Option<String> {
        let mut command = self.redo.pop()?;
        command.apply(doc);
        self.record(AuditAction::Redo, command.as_ref());
        let label = command.label();
        self.undo.push(command);
        Some(label)
    }

    pub fn undo_label(&self) -> Option<String> {
        self.undo.last().map(|c| c.label())
    }

    pub fn redo_label(&self) -> Option<String> {
        self.redo.last().map(|c| c.label())
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
