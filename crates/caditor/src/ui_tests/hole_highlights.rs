use caditor_render::Color;

use super::{CAMERA_SETTLE, Harness, extruded_plate, open_hole_on, pickable_described};
use crate::{editing::EditingCommand, model::Action, selection::Pickable};

fn drawn_colour(harness: &mut Harness, face: Pickable) -> Color {
    let built = harness.built();
    let pick = built.picks.id_of(face);
    built
        .scene
        .meshes
        .iter()
        .flat_map(|instance| instance.faces.iter())
        .find(|drawn| drawn.pick == pick)
        .map(|drawn| drawn.color)
        .expect("the face is drawn")
}

fn add_hole(harness: &mut Harness, top: Pickable) {
    open_hole_on(harness, top);
    harness.perform(Action::Editing(EditingCommand::CloseSolid));
    harness.settle();
}

struct Plate {
    top: Pickable,
    walls: [Pickable; 2],
    plain: Color,
}

impl Plate {
    fn with_two_holes(harness: &mut Harness) -> Self {
        let (_, top) = extruded_plate(harness);
        add_hole(harness, top);
        add_hole(harness, top);
        harness.select([]);
        harness.workspace.viewport.advance(CAMERA_SETTLE);
        harness.frame();

        let walls = [
            pickable_described(harness, "Extrude 1 › Hole 1 wall"),
            pickable_described(harness, "Extrude 1 › Hole 2 wall"),
        ];
        let plain = drawn_colour(harness, top);

        Self { top, walls, plain }
    }

    fn lit(&self, harness: &mut Harness) -> [bool; 3] {
        let [first, second] = self.walls;
        [first, second, self.top].map(|face| drawn_colour(harness, face) != self.plain)
    }
}

#[test]
fn choosing_one_hole_after_another_in_the_tree_moves_the_highlight_to_the_hole_chosen() {
    let mut harness = Harness::new();
    let plate = Plate::with_two_holes(&mut harness);

    assert_eq!(plate.lit(&mut harness), [false, false, false]);

    harness.click("Hole 1");
    harness.frame();
    let first_chosen = plate.lit(&mut harness);

    harness.click("Hole 2");
    harness.frame();
    let second_chosen = plate.lit(&mut harness);

    harness.click("Extrude 1");
    harness.frame();
    let body_chosen = plate.lit(&mut harness);

    assert_eq!(
        first_chosen,
        [true, false, false],
        "Hole 1 lights its own wall"
    );
    assert_eq!(second_chosen, [false, true, false], "Hole 2 takes over");
    assert_eq!(
        body_chosen,
        [true, true, true],
        "the body's row lights all of it"
    );
}
