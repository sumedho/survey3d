//! Named project records kept alongside the drawing: traverses, parcels,
//! alignments, raw field data and plan sheets.
//!
//! Each record has a stable [`RecordId`], so windows and scripts can refer to it
//! across edits, undo and reordering, and each change is its own undoable
//! operation (only the record that changed is copied into the history).

use std::collections::BTreeSet;
use std::ops::{Deref, DerefMut};

use serde::{Deserialize, Serialize, Serializer};
use survey_core::network::FieldData;
use survey_core::traverse::TraverseRecord;

use crate::alignment::AlignmentRecord;
use crate::image::ImageRecord;
use crate::parcel::ParcelRecord;
use crate::sheet::SheetRecord;

/// Identifier of a project record. Unique within a project (across all kinds).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RecordId(pub i64);

/// The record collections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    Traverse,
    Parcel,
    Alignment,
    FieldData,
    Sheet,
    Image,
}

impl RecordKind {
    pub const ALL: [RecordKind; 6] = [
        RecordKind::Traverse,
        RecordKind::Parcel,
        RecordKind::Alignment,
        RecordKind::FieldData,
        RecordKind::Sheet,
        RecordKind::Image,
    ];

    /// Storage key (also the collection name in scripts).
    pub fn key(self) -> &'static str {
        match self {
            RecordKind::Traverse => "traverses",
            RecordKind::Parcel => "parcels",
            RecordKind::Alignment => "alignments",
            RecordKind::FieldData => "field_data",
            RecordKind::Sheet => "sheets",
            RecordKind::Image => "images",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            RecordKind::Traverse => "traverse",
            RecordKind::Parcel => "parcel",
            RecordKind::Alignment => "alignment",
            RecordKind::FieldData => "field data",
            RecordKind::Sheet => "sheet",
            RecordKind::Image => "image",
        }
    }
}

/// A record with its id. Serialised flat (`{"id": 3, "name": …}`); records saved
/// before ids existed load with id 0 and are given one by the document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keyed<T> {
    #[serde(default)]
    pub id: RecordId,
    #[serde(flatten)]
    pub record: T,
}

impl<T> Deref for Keyed<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.record
    }
}

impl<T> DerefMut for Keyed<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.record
    }
}

/// Any record, as stored in the undo history.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnyRecord {
    Traverse(TraverseRecord),
    Parcel(ParcelRecord),
    Alignment(AlignmentRecord),
    // Observations can be large: the audit trail gets a summary.
    FieldData(#[serde(serialize_with = "field_data_summary")] Box<FieldData>),
    Sheet(SheetRecord),
    Image(ImageRecord),
}

fn field_data_summary<S: Serializer>(f: &FieldData, s: S) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct Summary<'a> {
        name: &'a str,
        format: &'a str,
        setups: usize,
        points: usize,
    }
    Summary {
        name: &f.name,
        format: &f.format,
        setups: f.setups.len(),
        points: f.points.len(),
    }
    .serialize(s)
}

impl AnyRecord {
    pub fn kind(&self) -> RecordKind {
        match self {
            AnyRecord::Traverse(_) => RecordKind::Traverse,
            AnyRecord::Parcel(_) => RecordKind::Parcel,
            AnyRecord::Alignment(_) => RecordKind::Alignment,
            AnyRecord::FieldData(_) => RecordKind::FieldData,
            AnyRecord::Sheet(_) => RecordKind::Sheet,
            AnyRecord::Image(_) => RecordKind::Image,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            AnyRecord::Traverse(r) => &r.name,
            AnyRecord::Parcel(r) => &r.name,
            AnyRecord::Alignment(r) => &r.name,
            AnyRecord::FieldData(r) => &r.name,
            AnyRecord::Sheet(r) => &r.name,
            AnyRecord::Image(r) => &r.name,
        }
    }
}

/// A record type stored in [`Records`].
pub trait Record: Clone + Sized {
    const KIND: RecordKind;
    fn list(records: &Records) -> &Vec<Keyed<Self>>;
    fn list_mut(records: &mut Records) -> &mut Vec<Keyed<Self>>;
    fn into_any(self) -> AnyRecord;
    fn from_any(any: AnyRecord) -> Option<Self>;
    fn name(&self) -> &str;
    fn kind_of(&self) -> RecordKind {
        Self::KIND
    }
}

macro_rules! record {
    ($ty:ty, $kind:ident, $field:ident, $wrap:expr, $unwrap:pat => $value:expr) => {
        impl Record for $ty {
            const KIND: RecordKind = RecordKind::$kind;
            fn list(records: &Records) -> &Vec<Keyed<Self>> {
                &records.$field
            }
            fn list_mut(records: &mut Records) -> &mut Vec<Keyed<Self>> {
                &mut records.$field
            }
            fn name(&self) -> &str {
                &self.name
            }
            fn into_any(self) -> AnyRecord {
                $wrap(self)
            }
            fn from_any(any: AnyRecord) -> Option<Self> {
                match any {
                    $unwrap => Some($value),
                    _ => None,
                }
            }
        }

        impl From<$ty> for AnyRecord {
            fn from(r: $ty) -> Self {
                r.into_any()
            }
        }
    };
}

record!(TraverseRecord, Traverse, traverses, AnyRecord::Traverse, AnyRecord::Traverse(r) => r);
record!(ParcelRecord, Parcel, parcels, AnyRecord::Parcel, AnyRecord::Parcel(r) => r);
record!(AlignmentRecord, Alignment, alignments, AnyRecord::Alignment, AnyRecord::Alignment(r) => r);
record!(FieldData, FieldData, field_data, |r| AnyRecord::FieldData(Box::new(r)), AnyRecord::FieldData(r) => *r);
record!(SheetRecord, Sheet, sheets, AnyRecord::Sheet, AnyRecord::Sheet(r) => r);
record!(ImageRecord, Image, images, AnyRecord::Image, AnyRecord::Image(r) => r);

/// All record collections, each in the user's order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Records {
    pub traverses: Vec<Keyed<TraverseRecord>>,
    pub parcels: Vec<Keyed<ParcelRecord>>,
    pub alignments: Vec<Keyed<AlignmentRecord>>,
    pub field_data: Vec<Keyed<FieldData>>,
    pub sheets: Vec<Keyed<SheetRecord>>,
    pub images: Vec<Keyed<ImageRecord>>,
}

impl Records {
    pub fn get<T: Record>(&self, id: RecordId) -> Option<&T> {
        T::list(self).iter().find(|r| r.id == id).map(|r| &r.record)
    }

    /// Largest id in use.
    pub(crate) fn max_id(&self) -> i64 {
        fn max<T>(v: &[Keyed<T>]) -> i64 {
            v.iter().map(|r| r.id.0).max().unwrap_or(0)
        }
        max(&self.traverses)
            .max(max(&self.parcels))
            .max(max(&self.alignments))
            .max(max(&self.field_data))
            .max(max(&self.sheets))
            .max(max(&self.images))
    }

    /// Gives every record without an id (from files saved before ids) the next
    /// free one; returns the collections that had any.
    pub(crate) fn assign_missing_ids(&mut self, next: &mut i64) -> BTreeSet<RecordKind> {
        fn assign<T>(v: &mut [Keyed<T>], next: &mut i64) -> bool {
            let mut any = false;
            for r in v.iter_mut().filter(|r| r.id.0 == 0) {
                r.id = RecordId(*next);
                *next += 1;
                any = true;
            }
            any
        }
        let mut kinds = BTreeSet::new();
        for (kind, any) in [
            (RecordKind::Traverse, assign(&mut self.traverses, next)),
            (RecordKind::Parcel, assign(&mut self.parcels, next)),
            (RecordKind::Alignment, assign(&mut self.alignments, next)),
            (RecordKind::FieldData, assign(&mut self.field_data, next)),
            (RecordKind::Sheet, assign(&mut self.sheets, next)),
            (RecordKind::Image, assign(&mut self.images, next)),
        ] {
            if any {
                kinds.insert(kind);
            }
        }
        kinds
    }

    /// Inserts, replaces (`Some`) or removes (`None`) a record; returns the previous
    /// value and its position. New records go at `at` (clamped) or the end.
    pub(crate) fn set(
        &mut self,
        kind: RecordKind,
        id: RecordId,
        value: Option<AnyRecord>,
        at: Option<usize>,
    ) -> Option<(usize, AnyRecord)> {
        fn set<T: Record>(
            records: &mut Records,
            id: RecordId,
            value: Option<AnyRecord>,
            at: Option<usize>,
        ) -> Option<(usize, AnyRecord)> {
            let list = T::list_mut(records);
            let pos = list.iter().position(|r| r.id == id);
            let value = value.and_then(T::from_any);
            match (value, pos) {
                (Some(record), Some(i)) => {
                    let old = std::mem::replace(&mut list[i].record, record);
                    Some((i, old.into_any()))
                }
                (Some(record), None) => {
                    let i = at.unwrap_or(list.len()).min(list.len());
                    list.insert(i, Keyed { id, record });
                    None
                }
                (None, Some(i)) => Some((i, list.remove(i).record.into_any())),
                (None, None) => None,
            }
        }
        match kind {
            RecordKind::Traverse => set::<TraverseRecord>(self, id, value, at),
            RecordKind::Parcel => set::<ParcelRecord>(self, id, value, at),
            RecordKind::Alignment => set::<AlignmentRecord>(self, id, value, at),
            RecordKind::FieldData => set::<FieldData>(self, id, value, at),
            RecordKind::Sheet => set::<SheetRecord>(self, id, value, at),
            RecordKind::Image => set::<ImageRecord>(self, id, value, at),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyed_serialises_flat_and_reads_old_records() {
        let sheet = Keyed {
            id: RecordId(7),
            record: SheetRecord::default(),
        };
        let json = serde_json::to_value(&sheet).expect("json");
        assert_eq!(json["id"], 7);
        assert_eq!(json["name"], "Sheet 1");
        let back: Keyed<SheetRecord> = serde_json::from_value(json).expect("parse");
        assert_eq!(back, sheet);
        // A record saved before ids existed.
        let old: Vec<Keyed<ParcelRecord>> =
            serde_json::from_str(r#"[{"name":"Lot 1","boundary":4}]"#).expect("old parcel");
        assert_eq!(old[0].id, RecordId(0));
        assert_eq!(old[0].name, "Lot 1");
    }

    #[test]
    fn set_inserts_replaces_and_removes_in_place() {
        let mut r = Records::default();
        let sheet = |name: &str| {
            Some(AnyRecord::Sheet(SheetRecord {
                name: name.into(),
                ..Default::default()
            }))
        };
        assert!(r.set(RecordKind::Sheet, RecordId(1), sheet("A"), None).is_none());
        r.set(RecordKind::Sheet, RecordId(2), sheet("B"), None);
        r.set(RecordKind::Sheet, RecordId(3), sheet("C"), None);
        let (i, old) = r
            .set(RecordKind::Sheet, RecordId(2), None, None)
            .expect("removed");
        assert_eq!((i, old.name()), (1, "B"));
        // Undoing the delete puts it back where it was.
        r.set(RecordKind::Sheet, RecordId(2), Some(old), Some(i));
        let names: Vec<&str> = r.sheets.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["A", "B", "C"]);
        let (_, old) = r
            .set(RecordKind::Sheet, RecordId(3), sheet("C2"), None)
            .expect("replaced");
        assert_eq!(old.name(), "C");
        assert_eq!(
            r.get::<SheetRecord>(RecordId(3)).map(|s| s.name.as_str()),
            Some("C2")
        );

        let mut next = 10;
        r.parcels.push(Keyed {
            id: RecordId(0),
            record: ParcelRecord::default(),
        });
        let kinds = r.assign_missing_ids(&mut next);
        assert_eq!((r.parcels[0].id, next), (RecordId(10), 11));
        assert_eq!(kinds, [RecordKind::Parcel].into_iter().collect());
        assert_eq!(r.max_id(), 10);
    }
}
