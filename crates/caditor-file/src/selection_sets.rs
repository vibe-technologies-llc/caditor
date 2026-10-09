use caditor_document::{
    Document, FIRST_UNSTORABLE_ID, FeatureId, MAX_SELECTION_SETS, MAX_SET_MEMBERS,
    MAX_SET_NAME_CHARS, SelectionSet, SelectionSets, SetMember, set_name,
};
use serde::{Deserialize, Serialize};

use crate::format::{
    EdgeRecord, FaceRecord, Lenient, edge_record, face_record, restore_edge, restore_face,
};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct SelectionSetsRecord {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sets: Vec<Lenient<SelectionSetRecord>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SelectionSetRecord {
    pub name: String,
    pub members: Vec<Lenient<SetMemberRecord>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SetMemberRecord {
    Body(u64),
    Face { body: u64, face: FaceRecord },
    Edge { body: u64, edge: EdgeRecord },
}

fn member_record(member: &SetMember) -> SetMemberRecord {
    match member {
        SetMember::Body(body) => SetMemberRecord::Body(body.raw()),
        SetMember::Face { body, face } => SetMemberRecord::Face {
            body: body.raw(),
            face: face_record(face),
        },
        SetMember::Edge { body, edge } => SetMemberRecord::Edge {
            body: body.raw(),
            edge: edge_record(edge),
        },
    }
}

pub(crate) fn selection_sets_record_of(sets: &SelectionSets) -> SelectionSetsRecord {
    SelectionSetsRecord {
        sets: sets
            .sets
            .iter()
            .map(|set| {
                Lenient::Read(SelectionSetRecord {
                    name: set.name.clone(),
                    members: set
                        .members
                        .iter()
                        .map(|member| Lenient::Read(member_record(member)))
                        .collect(),
                })
            })
            .collect(),
    }
}

pub(crate) fn selection_sets_record(document: &Document) -> Option<SelectionSetsRecord> {
    let sets = document.selection_sets();
    (!sets.is_empty()).then(|| selection_sets_record_of(sets))
}

fn storable(raw: u64) -> Option<FeatureId> {
    (raw < FIRST_UNSTORABLE_ID).then(|| FeatureId::from_raw(raw))
}

fn restore_member(record: &SetMemberRecord) -> Option<SetMember> {
    Some(match record {
        SetMemberRecord::Body(body) => SetMember::Body(storable(*body)?),
        SetMemberRecord::Face { body, face } => SetMember::Face {
            body: storable(*body)?,
            face: restore_face(&face.face, face.origin, face.copy, &face.neighbours)?,
        },
        SetMemberRecord::Edge { body, edge } => SetMember::Edge {
            body: storable(*body)?,
            edge: restore_edge(edge)?,
        },
    })
}

fn fitting_set_name(name: &str, taken: &SelectionSets, issues: &mut Vec<String>) -> String {
    let mut name = set_name(name);
    if name.is_empty() {
        let numbered = taken.unused_name();
        issues.push(format!(
            "A selection set had no name, so it is called “{numbered}”."
        ));
        return numbered;
    }
    if name.chars().count() > MAX_SET_NAME_CHARS {
        name = name
            .chars()
            .take(MAX_SET_NAME_CHARS)
            .collect::<String>()
            .trim_end()
            .to_owned();
        issues.push(format!(
            "The name of the selection set “{name}” was longer than {MAX_SET_NAME_CHARS} \
             characters, so its end was cut off."
        ));
    }
    if !taken.is_taken(&name) {
        return name;
    }
    let room = MAX_SET_NAME_CHARS - 6;
    let base: String = name.chars().take(room).collect();
    let numbered = (2..)
        .map(|number| format!("{base} {number}"))
        .find(|candidate| !taken.is_taken(candidate))
        .unwrap_or_else(|| name.clone());
    issues.push(format!(
        "Two selection sets were called “{name}”, so one is called “{numbered}”."
    ));
    numbered
}

fn restore_members(record: &SelectionSetRecord, issues: &mut Vec<String>) -> Vec<SetMember> {
    let mut unreadable = 0_usize;
    let mut members: Vec<SetMember> = Vec::new();
    for member in &record.members {
        match member {
            Lenient::Read(member) => match restore_member(member) {
                Some(member) => members.push(member),
                None => unreadable += 1,
            },
            Lenient::Unreadable(_) => unreadable += 1,
        }
    }
    let name = &record.name;
    match unreadable {
        0 => {}
        1 => issues.push(format!(
            "One face, edge or body in the selection set “{name}” could not be read, so it was \
             left out."
        )),
        count => issues.push(format!(
            "{count} faces, edges or bodies in the selection set “{name}” could not be read, so \
             they were left out."
        )),
    }
    if members.len() > MAX_SET_MEMBERS {
        members.truncate(MAX_SET_MEMBERS);
        issues.push(format!(
            "A selection set holds at most {MAX_SET_MEMBERS} faces, edges and bodies, so the \
             rest of “{name}” was left out."
        ));
    }
    members
}

pub(crate) fn restore_selection_sets(
    record: SelectionSetsRecord,
    issues: &mut Vec<String>,
) -> SelectionSets {
    let mut sets = SelectionSets::default();
    for set in record.sets {
        let Lenient::Read(set) = set else {
            issues.push("A selection set could not be read, so it was left out.".to_owned());
            continue;
        };
        if sets.sets.len() >= MAX_SELECTION_SETS {
            issues.push(format!(
                "A model keeps at most {MAX_SELECTION_SETS} selection sets, so the rest were \
                 left out."
            ));
            break;
        }
        let members = restore_members(&set, issues);
        if members.is_empty() {
            issues.push(format!(
                "The selection set “{}” held nothing that could be read, so it was left out.",
                set.name
            ));
            continue;
        }
        let name = fitting_set_name(&set.name, &sets, issues);
        sets.sets.push(SelectionSet { name, members });
    }
    sets
}
