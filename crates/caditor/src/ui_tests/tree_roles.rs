use std::collections::BTreeMap;

use caditor_document::PrincipalGeometry;
use egui::{
    Key, Modifiers,
    accesskit::{Node, NodeId, Role},
};

use super::{Harness, add_peg, extruded_plate, run_from_palette};
use crate::{feature_tree::TREE_NAME, parameter_table::MODEL_PARAMETERS};

const PRINCIPAL: &str = crate::principal_tree::GROUP_TITLE;

fn parents(harness: &Harness) -> BTreeMap<NodeId, NodeId> {
    harness
        .accessible
        .iter()
        .flat_map(|(id, node)| node.children().iter().map(|child| (*child, *id)))
        .collect()
}

fn descends_from(harness: &Harness, node: NodeId, ancestor: NodeId) -> bool {
    let parents = parents(harness);
    let mut current = node;
    while let Some(parent) = parents.get(&current) {
        if *parent == ancestor {
            return true;
        }
        current = *parent;
    }
    false
}

fn row_of(harness: &Harness, node: NodeId) -> Option<NodeId> {
    let parents = parents(harness);
    let mut current = node;
    while let Some(parent) = parents.get(&current) {
        let is_row = harness
            .accessible
            .iter()
            .any(|(id, node)| id == parent && node.role() == Role::Row);
        if is_row {
            return Some(*parent);
        }
        current = *parent;
    }
    None
}

fn nodes_of<'a>(harness: &'a Harness, role: Role, name: &str) -> Vec<(NodeId, &'a Node)> {
    harness
        .accessible
        .iter()
        .filter(|(_, node)| node.role() == role && node.label() == Some(name))
        .map(|(id, node)| (*id, node))
        .collect()
}

fn only<'a>(harness: &'a Harness, role: Role, name: &str) -> (NodeId, &'a Node) {
    let mut found = nodes_of(harness, role, name);
    assert_eq!(found.len(), 1, "{role:?} named {name}");
    found.remove(0)
}

fn item<'a>(harness: &'a Harness, name: &str) -> &'a Node {
    only(harness, Role::TreeItem, name).1
}

fn levels(harness: &Harness, name: &str) -> Vec<usize> {
    let mut levels: Vec<usize> = nodes_of(harness, Role::TreeItem, name)
        .into_iter()
        .filter_map(|(_, node)| node.level())
        .collect();
    levels.sort_unstable();
    levels
}

fn read(harness: &mut Harness) {
    harness.read_screen();
}

#[test]
fn the_feature_tree_is_a_tree_of_items_with_their_level_expansion_and_selection() {
    let mut harness = Harness::new();
    read(&mut harness);

    let (tree, node) = only(&harness, Role::Tree, TREE_NAME);
    let tree_name = node.label().map(str::to_owned);
    let items: Vec<NodeId> = harness
        .accessible
        .iter()
        .filter(|(_, node)| node.role() == Role::TreeItem)
        .map(|(id, _)| *id)
        .collect();
    let in_one_tree = items.iter().all(|id| descends_from(&harness, *id, tree));
    let plain = item(&harness, "Base sketch");
    let first = (plain.level(), plain.is_selected(), plain.is_expanded());
    let principal = item(&harness, PRINCIPAL);
    let group = (principal.level(), principal.is_expanded());

    harness.click("Base sketch");
    read(&mut harness);
    let selected = item(&harness, "Base sketch").is_selected();
    let other_selected = item(&harness, "Side sketch").is_selected();

    harness.click_button("Show details of Base sketch");
    read(&mut harness);
    let opened = item(&harness, "Base sketch").is_expanded();
    harness.click_button("Hide details of Base sketch");
    read(&mut harness);
    let closed = item(&harness, "Base sketch").is_expanded();

    harness.click_button(&format!("Show details of {PRINCIPAL}"));
    read(&mut harness);
    let principal_opened = item(&harness, PRINCIPAL).is_expanded();
    let geometry_levels: Vec<Option<usize>> = PrincipalGeometry::ALL
        .iter()
        .map(|geometry| item(&harness, geometry.name()).level())
        .collect();

    assert_eq!(tree_name.as_deref(), Some(TREE_NAME));
    assert!(items.len() >= 2);
    assert!(in_one_tree);
    assert_eq!(first, (Some(1), Some(false), Some(false)));
    assert_eq!(group, (Some(1), Some(false)));
    assert_eq!(selected, Some(true));
    assert_eq!(other_selected, Some(false));
    assert_eq!(opened, Some(true));
    assert_eq!(closed, Some(false));
    assert_eq!(principal_opened, Some(true));
    assert_eq!(geometry_levels, vec![Some(2); PrincipalGeometry::ALL.len()]);
}

#[test]
fn bodies_are_items_one_level_below_their_group_beside_the_feature_that_made_them() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.settle();
    read(&mut harness);
    let bodies = crate::bodies_tree::GROUP_TITLE;
    let header = format!("{bodies} (1)");

    let closed = item(&harness, &header).is_expanded();
    let hidden_while_closed = levels(&harness, "Extrude 1");
    harness.click_button(&format!("Show details of {header}"));
    read(&mut harness);
    let opened = item(&harness, &header).is_expanded();
    let (tree, _) = only(&harness, Role::Tree, TREE_NAME);
    let body_rows = nodes_of(&harness, Role::TreeItem, "Extrude 1");
    let in_the_tree = body_rows
        .iter()
        .all(|(id, _)| descends_from(&harness, *id, tree));

    assert_eq!(item(&harness, &header).level(), Some(1));
    assert_eq!(closed, Some(false));
    assert_eq!(hidden_while_closed, [1]);
    assert_eq!(opened, Some(true));
    assert_eq!(levels(&harness, "Extrude 1"), [1, 2]);
    assert!(in_the_tree);
    assert!(
        body_rows
            .iter()
            .all(|(_, node)| node.is_expanded().is_some() == (node.level() == Some(1)))
    );
}

#[test]
fn grouped_features_sit_one_level_below_their_folder_which_reads_expanded_or_folded() {
    let mut harness = Harness::new();
    let ids: Vec<_> = harness
        .document()
        .features()
        .map(|feature| feature.id())
        .collect();
    harness.workspace.panels.choose_all(&ids);
    harness.frame();
    harness.key(Key::G, Modifiers::COMMAND);
    harness.settle();
    harness.frame();
    harness.replace_text("Profiles");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.settle();
    read(&mut harness);

    let folder = item(&harness, "Profiles");
    let open = (folder.level(), folder.is_expanded());
    let members = (
        levels(&harness, "Base sketch"),
        levels(&harness, "Side sketch"),
    );

    harness.click_button("Hide details of Profiles");
    harness.frame();
    harness.frame();
    read(&mut harness);
    let folded = item(&harness, "Profiles").is_expanded();
    let gone = nodes_of(&harness, Role::TreeItem, "Base sketch").len();

    assert_eq!(open, (Some(1), Some(true)));
    assert_eq!(members, (vec![2], vec![2]));
    assert_eq!(folded, Some(false));
    assert_eq!(gone, 0);
}

#[test]
fn body_groups_nest_their_items_a_level_per_group() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);
    harness.settle();
    harness.workspace.panels.choose_all(&[plate, peg]);
    harness.frame();
    run_from_palette(&mut harness, "group bodies");
    harness.settle();
    harness.frame();
    harness.replace_text("Fixtures");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.settle();
    harness.workspace.panels.choose_only(peg);
    harness.frame();
    run_from_palette(&mut harness, "group bodies");
    harness.settle();
    harness.frame();
    harness.replace_text("Pins");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.settle();
    read(&mut harness);

    let outer = item(&harness, "Fixtures");
    let inner = item(&harness, "Pins");
    let plate_name = harness.document().body_name(plate).unwrap().into_owned();
    let peg_name = harness.document().body_name(peg).unwrap().into_owned();

    assert_eq!((outer.level(), outer.is_expanded()), (Some(2), Some(true)));
    assert_eq!((inner.level(), inner.is_expanded()), (Some(3), Some(true)));
    assert!(levels(&harness, &plate_name).contains(&3));
    assert!(levels(&harness, &peg_name).contains(&4));
}

#[test]
fn the_parameter_tables_are_tables_of_rows_and_cells_with_column_headers() {
    let mut harness = Harness::new();
    read(&mut harness);

    let (table, node) = only(&harness, Role::Table, "Parameters");
    let rows = harness
        .accessible
        .iter()
        .filter(|(id, node)| node.role() == Role::Row && descends_from(&harness, *id, table))
        .count();
    let headers: Vec<String> = ["Name", "Expression", "Value"]
        .into_iter()
        .filter(|caption| {
            nodes_of(&harness, Role::ColumnHeader, caption)
                .iter()
                .any(|(id, _)| descends_from(&harness, *id, table))
        })
        .map(str::to_owned)
        .collect();
    let width_cells = harness
        .accessible
        .iter()
        .filter(|(id, node)| node.role() == Role::Cell && descends_from(&harness, *id, table))
        .filter_map(|(_, node)| node.row_index().zip(node.column_index()))
        .collect::<Vec<_>>();
    let named = harness
        .accessible
        .iter()
        .filter(|(_, node)| node.role() == Role::TextInput && node.value() == Some("width"))
        .all(|(id, _)| descends_from(&harness, *id, table));
    let same_row = {
        let name = harness
            .accessible
            .iter()
            .find(|(_, node)| node.role() == Role::TextInput && node.value() == Some("width"))
            .and_then(|(id, _)| row_of(&harness, *id));
        let expression = harness
            .accessible
            .iter()
            .find(|(_, node)| node.role() == Role::TextInput && node.value() == Some("40 mm"))
            .and_then(|(id, _)| row_of(&harness, *id));
        name.is_some() && name == expression
    };

    assert_eq!(node.row_count(), Some(3));
    assert_eq!(node.column_count(), Some(4));
    assert_eq!(rows, 3);
    assert_eq!(headers, ["Name", "Expression", "Value"]);
    assert!(width_cells.contains(&(1, 0)));
    assert!(width_cells.contains(&(1, 1)));
    assert!(width_cells.contains(&(1, 2)));
    assert!(named);
    assert!(same_row);
    assert!(nodes_of(&harness, Role::Table, MODEL_PARAMETERS).is_empty());
}
