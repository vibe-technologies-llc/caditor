use caditor_document::{
    AxisReference, AxisSide, BodyOperation, Document, Extrude, ExtrudeEnd, ExtrudeExtent, Feature,
    FeatureId, PlaneReference, RegionChoice, Revolve, RevolveAxis, RevolveExtent, SolidFeature,
    SolidStart, Sweep, SweepPath, Transaction, Wall, along_plane, capitalized, describe_axis,
    describe_origin, displayed_axis,
};
use caditor_expression::{Dimension, Expression};
use caditor_geometry::Point2;
use caditor_kernel::{SweepOrientation, WallSide};
use caditor_sketch::{Entity, EntityId, Reference};
use egui::{Id, Ui};

use crate::{
    commands::Command,
    datum_tools,
    editing::EditingCommand,
    feature_fields::{
        self, AxisRow, Choice, ListChoosing, MISSING_BODY, MISSING_SKETCH, Picker, Quantity,
        REVERSE_DIRECTION, Rule, Segment, Shown,
    },
    feature_tree::count,
    field, hole_panel, icons,
    model::{Action, Model},
    move_manipulator::Reach,
    plane_selector,
    reference_picking::{self, Picking, Side, Slot},
    reversing, scene,
    selection::{self, Pickable, Selection},
    sketch_placement::{self, FaceChoice},
    solid_tools::{self, DEFAULT_BACKWARD_ANGLE, DEFAULT_PARTIAL_ANGLE},
    sweep_tools, widgets,
};

struct PlaneRow<'a> {
    caption: &'a str,
    current: Option<&'a PlaneReference>,
    picker: Picker<'a>,
    remove: Option<&'a str>,
}

const EDIT_SKETCH_HOVER: &str = "Show and edit the sketch this sweeps, to change its profile";
const FULL_TURN_DEGREES: f64 = 360.0;
const WHOLE_PROFILE: &str = "Turn the whole profile, which must lie on one side of the axis";
const ONE_SIDE_OF_AXIS: &str =
    "Turn only the part of the profile on one side of the axis, cut where the axis crosses it";
pub const KEEP_OTHER_SIDE: &str = "Keep the other side of the axis";
pub const AXIS: &str = "Axis";
pub const ALSO_CUTS: &str = "Also cuts";
pub const ADD_CUT_BODY: &str = "Add another body";
const START_OFFSET: &str = "Start offset";
pub const PATH: &str = "Path";
pub const ORIENTATION: &str = "Orientation";
pub const SQUARE_TO_PATH: &str = "Square to path";
pub const PARALLEL: &str = "Parallel";
const SQUARE_TO_PATH_HOVER: &str =
    "Turn the profile with the path so it keeps its angle to the path, as a pipe bends";
const PARALLEL_HOVER: &str =
    "Keep the profile parallel to its sketch, only moving it along the path";
const FOLLOW_SELECTED_PATH: &str = "Follow the selected curves or edges instead";
const CLEAR_PATH: &str = "Clear the path";
pub const END_OFFSET: &str = "Past the face";
const FORWARD_END_OFFSET: &str = "Forward past face";
const BACKWARD_END_OFFSET: &str = "Backward past face";

pub fn end_offset_caption(reach: Reach) -> &'static str {
    match reach {
        Reach::Only | Reach::Symmetric => END_OFFSET,
        Reach::Forward => FORWARD_END_OFFSET,
        Reach::Backward => BACKWARD_END_OFFSET,
    }
}
pub const TAPER: &str = "Taper";
pub const TURN_UP_TO: &str = "Up to face";
pub const SQUARE: &str = "Square to the sketch";
pub const ALONG: &str = "Along an edge or axis";
const NO_DIRECTION_SELECTED: &str =
    "Select a straight edge, an axis or a sketch line made before this feature";
pub const THIN_WALL: &str = "Thin wall";
pub const WALL_THICKNESS: &str = "Thickness";
const FILL_SOLID: &str = "Fill the closed regions of the sketch";
const FILL_WALL: &str = "Thicken the curves of the sketch into a wall, whether they close or not";
const WALL_CURVES: &str = "The wall follows every curve of the sketch";
const WALL_INSIDE: &str =
    "Thicken towards the enclosed side of a closed outline, or the side an open chain bends around";
const WALL_OUTSIDE: &str = "Thicken away from the inside";
const WALL_CENTRED: &str = "Thicken half the thickness to each side of the curves";
pub const CLEAR_REGIONS: &str = "Clear";
const CLEAR_REGIONS_HOVER: &str = "Choose no region, then click in the view the regions to sweep";
const START_BY_OFFSET: &str = "Sketch plane, offset";
const START_ON_FACE: &str = "Face or plane";
const THROUGH_ALL_NEEDS_A_CUT: &str = "Through all cuts into a body or intersects with it; choose Remove from body or Intersect \
     with body first";
const UP_TO_NEXT_NEEDS_A_BODY: &str = "Up to next stops at the body this feature changes; choose Add, Remove or Intersect with a \
     body first";
const NO_TARGET_SELECTED: &str =
    "Select a flat face or a plane made before this feature, then use it";
const CURVED_TARGET: &str = "The selected face is curved; only a flat face or plane can be an end";
const SIDES_CHANGED: &str = "The extrusion no longer has that end; choose the face again";
pub const FAR_PAST_THE_BODY: &str = "This distance reaches far past the body. Through all cuts through \
     the whole body whatever its size, and keeps doing so when the body grows.";
pub const CUT_THROUGH_ALL: &str = "Cut through all instead";
const FAR_PAST_THE_BODY_TIMES: f64 = 10.0;

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: &'a Feature,
    solid: &'a SolidFeature,
    actions: &'a mut Vec<Action>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndKind {
    Distance,
    ThroughAll,
    UpToNext,
    UpToFace,
}

impl EndKind {
    const ALL: [Self; 4] = [
        Self::Distance,
        Self::ThroughAll,
        Self::UpToNext,
        Self::UpToFace,
    ];

    fn of(end: &ExtrudeEnd) -> Self {
        match end {
            ExtrudeEnd::Distance(_) => Self::Distance,
            ExtrudeEnd::ThroughAll => Self::ThroughAll,
            ExtrudeEnd::UpToNext { .. } => Self::UpToNext,
            ExtrudeEnd::UpToFace { .. } | ExtrudeEnd::UpToSurface { .. } => Self::UpToFace,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Distance => "Distance",
            Self::ThroughAll => "Through all",
            Self::UpToNext => "Up to next",
            Self::UpToFace => "Up to face",
        }
    }
}

pub const DISTANCE: &str = "Distance";
pub const TOTAL_DISTANCE: &str = "Total distance";
pub const FORWARD_DISTANCE: &str = "Forward distance";
pub const BACKWARD_DISTANCE: &str = "Backward distance";

pub const ANGLE: &str = "Angle";
pub const TOTAL_ANGLE: &str = "Total angle";
pub const FORWARD_ANGLE: &str = "Forward";
pub const BACKWARD_ANGLE: &str = "Backward";

pub fn captioned(solid: &SolidFeature) -> Vec<(&'static str, &Expression)> {
    match solid {
        SolidFeature::Extrude(extrude) => extent_captioned(&extrude.extent),
        SolidFeature::Revolve(revolve) => match &revolve.extent {
            RevolveExtent::Full | RevolveExtent::UpTo { .. } => Vec::new(),
            RevolveExtent::OneSide { angle, .. } => vec![(ANGLE, angle)],
            RevolveExtent::Symmetric { angle } => vec![(TOTAL_ANGLE, angle)],
            RevolveExtent::TwoSides { forward, backward } => {
                vec![(FORWARD_ANGLE, forward), (BACKWARD_ANGLE, backward)]
            }
        },
        SolidFeature::Sweep(_) => Vec::new(),
    }
}

fn extent_captioned(extent: &ExtrudeExtent) -> Vec<(&'static str, &Expression)> {
    match extent {
        ExtrudeExtent::OneSide { end, .. } => end_captioned(end, DISTANCE, END_OFFSET),
        ExtrudeExtent::Symmetric { distance } => vec![(TOTAL_DISTANCE, distance)],
        ExtrudeExtent::TwoSides { forward, backward } => {
            let mut fields = end_captioned(forward, FORWARD_DISTANCE, FORWARD_END_OFFSET);
            fields.extend(end_captioned(
                backward,
                BACKWARD_DISTANCE,
                BACKWARD_END_OFFSET,
            ));
            fields
        }
    }
}

fn end_captioned<'a>(
    end: &'a ExtrudeEnd,
    distance: &'static str,
    offset: &'static str,
) -> Vec<(&'static str, &'a Expression)> {
    let reach = end.distance().map(|value| (distance, value));
    let past = end.offset().map(|value| (offset, value));
    reach.into_iter().chain(past).collect()
}

struct EndRows<'a> {
    end: &'a str,
    distance: &'a str,
    face: &'a str,
    offset: &'a str,
    salt: &'a str,
    side: Side,
    rule: Rule,
}

impl Panel<'_> {
    fn document(&self) -> &Document {
        self.model.document()
    }

    fn id(&self) -> FeatureId {
        self.feature.id()
    }

    fn change(&self, solid: SolidFeature) -> Result<Transaction, String> {
        change(self.model, self.id(), solid)
    }

    fn default_distance(&self) -> Expression {
        self.model
            .length_unit()
            .default_length(solid_tools::DEFAULT_DISTANCE)
    }

    fn picking(&self, ui: &Ui, slot: Slot) -> bool {
        reference_picking::current(ui.ctx()).is_some_and(|picking| picking.is_for(self.id(), slot))
    }

    fn plane_row(
        &mut self,
        ui: &mut Ui,
        row: PlaneRow<'_>,
        rebuild: impl Fn(PlaneReference) -> SolidFeature,
    ) -> bool {
        let model = self.model;
        let feature = self.id();
        let mut actions = Vec::new();
        let removed = plane_selector::row(
            ui,
            model,
            plane_selector::Row {
                caption: row.caption,
                id: Id::new(("plane-row", row.caption, feature)),
                planes: datum_tools::listed_planes(model.document(), feature),
                current: row.current,
                picker: row.picker,
                remove: row.remove,
            },
            |plane| self.change(rebuild(plane.clone())),
            &mut actions,
        );
        self.actions.extend(actions);
        removed
    }

    fn combo(
        &mut self,
        ui: &mut Ui,
        salt: &str,
        selected: impl Into<egui::WidgetText>,
        options: impl FnOnce(&Self) -> Vec<Choice>,
    ) {
        let chosen =
            feature_fields::combo(ui, Id::new((salt, self.id())), selected, || options(self));
        self.actions.extend(chosen);
    }

    fn expression(
        &mut self,
        ui: &mut Ui,
        caption: &str,
        salt: &str,
        expression: &Expression,
        (dimension, rule): (Dimension, Rule),
        rebuild: impl Fn(Expression) -> SolidFeature,
    ) {
        let model = self.model;
        let id = self.id();
        let quantity = Quantity {
            feature: id,
            id: Id::new(("solid-field", salt, id)),
            expression,
            dimension,
            rule,
        };
        let drafting =
            feature_fields::expression_row_drafting(ui, model, caption, quantity, |parsed| {
                change(model, id, rebuild(parsed))
            });
        let announced = drafting
            .committed
            .as_ref()
            .and_then(|committed| reversing::announcement(model.document(), committed));
        self.actions.extend(drafting.into_actions(id));
        self.actions.extend(announced.map(Action::Inform));
    }

    fn sketch_row(&mut self, ui: &mut Ui) {
        widgets::caption(ui, "Sketch");
        let current = self.solid.sketch();
        let model = self.model;
        ui.horizontal(|ui| {
            let name = feature_fields::feature_name(model.document(), current);
            let name_exists = name.is_some();
            let text = feature_fields::combo_text(ui, name, MISSING_SKETCH);
            self.combo(ui, "sketch", text, |panel| {
                let end = panel.document().feature_index(panel.id()).unwrap_or(0);
                panel
                    .document()
                    .features()
                    .take(end)
                    .filter(|candidate| candidate.kind.sketch().is_some())
                    .map(|candidate| Choice {
                        label: candidate.name.clone(),
                        selected: candidate.id() == current,
                        change: panel
                            .change(with_sketch(panel.solid, candidate.id()))
                            .map(Action::Apply),
                    })
                    .collect()
            });
            if name_exists {
                let button = widgets::icon_button(ui, icons::EDIT, hole_panel::EDIT_SKETCH);
                if button.on_hover_text(EDIT_SKETCH_HOVER).clicked() {
                    self.actions
                        .push(Action::Editing(EditingCommand::Enter(current)));
                }
            }
        });
        ui.end_row();
    }

    fn regions_row(&mut self, ui: &mut Ui, opened: bool) {
        widgets::caption(ui, "Regions");
        let model = self.model;
        let document = model.document();
        let evaluation = model.evaluation();
        let regions = selection::swept_regions(document, evaluation, self.id());
        let all = matches!(self.solid.regions(), RegionChoice::All);
        let chosen_count = regions.map_or(0, |(_, regions)| {
            scene::chosen_regions(self.solid.regions(), regions).len()
        });
        let mut change = None;
        ui.vertical(|ui| {
            let choices = [
                (
                    "All closed",
                    "Sweep every region that is not a hole of another",
                ),
                ("Chosen", "Sweep only the regions you choose in the view"),
            ];
            match widgets::segmented(ui, &choices, usize::from(!all)) {
                Some(0) => {
                    change =
                        Some(self.change(solid_tools::with_regions(self.solid, RegionChoice::All)));
                }
                Some(_) => {
                    if let Some((_, regions)) = regions {
                        let keys = scene::chosen_regions(&RegionChoice::All, regions);
                        change = Some(self.change(solid_tools::with_regions(
                            self.solid,
                            RegionChoice::Chosen(scene::region_references(&keys, regions)),
                        )));
                    }
                }
                None => {}
            }
            ui.horizontal(|ui| {
                if !all {
                    let chosen = format!("{} chosen", count(chosen_count, "region", "regions"));
                    ui.label(widgets::muted(chosen, ui));
                }
                if chosen_count > 0 {
                    let clear = widgets::small_button(
                        ui,
                        icons::command(Command::ClearChosenRegions),
                        CLEAR_REGIONS,
                    );
                    if ui.add(clear).on_hover_text(CLEAR_REGIONS_HOVER).clicked() {
                        change = Some(solid_tools::clear_regions(self.model, self.feature));
                    }
                }
            });
            let chosen = feature_fields::list_in_view(
                ui,
                opened,
                "Click regions in the view to include or leave them out.",
                "Show the regions of the sketch so you can click them",
            );
            match chosen {
                Some(ListChoosing::Start) => self
                    .actions
                    .push(Action::Editing(EditingCommand::OpenSolid(self.id()))),
                Some(ListChoosing::Stop) => {
                    self.actions
                        .push(Action::Editing(EditingCommand::CloseSolid));
                }
                None => {}
            }
        });
        ui.end_row();
        if let Some(change) = change {
            self.actions
                .push(feature_fields::applied(&self.feature.name, change));
        }
    }

    fn end_choices(
        &self,
        extrude: &Extrude,
        end: &ExtrudeEnd,
        side: Side,
        rebuild: &dyn Fn(ExtrudeEnd) -> ExtrudeExtent,
    ) -> Vec<Choice> {
        let current = EndKind::of(end);
        EndKind::ALL
            .into_iter()
            .map(|kind| {
                let candidate = match kind {
                    EndKind::Distance => Ok(ExtrudeEnd::Distance(
                        end.distance()
                            .cloned()
                            .unwrap_or_else(|| self.default_distance()),
                    )),
                    EndKind::ThroughAll => match extrude.operation {
                        BodyOperation::Remove(_) | BodyOperation::Intersect(_) => {
                            Ok(ExtrudeEnd::ThroughAll)
                        }
                        BodyOperation::NewBody | BodyOperation::Add(_) => {
                            Err(THROUGH_ALL_NEEDS_A_CUT.to_owned())
                        }
                    },
                    EndKind::UpToNext => extrude
                        .operation
                        .target()
                        .map(|_| ExtrudeEnd::up_to_next().with_offset(end.offset().cloned()))
                        .ok_or_else(|| UP_TO_NEXT_NEEDS_A_BODY.to_owned()),
                    EndKind::UpToFace => {
                        selected_end(self.model, self.selection, self.id(), end.offset().cloned())
                    }
                };
                let change = match candidate {
                    Ok(candidate) => self
                        .change(with_extent_of(extrude, rebuild(candidate)))
                        .map(Action::Apply),
                    Err(_) if kind == EndKind::UpToFace => Ok(Action::Editing(
                        EditingCommand::Pick(Picking::new(self.id(), Slot::ExtrudeTarget(side))),
                    )),
                    Err(reason) => Err(reason),
                };
                Choice {
                    label: kind.label().to_owned(),
                    selected: kind == current,
                    change,
                }
            })
            .collect()
    }

    fn end_rows(
        &mut self,
        ui: &mut Ui,
        rows: &EndRows<'_>,
        extrude: &Extrude,
        end: &ExtrudeEnd,
        rebuild: &dyn Fn(ExtrudeEnd) -> ExtrudeExtent,
    ) {
        widgets::caption(ui, rows.end);
        let salt = format!("{}-end", rows.salt);
        self.combo(ui, &salt, EndKind::of(end).label(), |panel| {
            panel.end_choices(extrude, end, rows.side, rebuild)
        });
        ui.end_row();
        let slot = Slot::ExtrudeTarget(rows.side);
        let kept_offset = end.offset().cloned();
        let picker = Picker {
            feature: self.id(),
            slot,
            selected: feature_fields::offered_change(
                ui.ctx(),
                self.model,
                self.selection,
                (self.id(), slot),
                || target_change(self.model, self.selection, self.id(), extrude, rows.side),
            ),
            hover: "Run up to the selected face or plane instead",
        };
        match end {
            ExtrudeEnd::Distance(distance) => {
                let model = self.model;
                self.expression(
                    ui,
                    rows.distance,
                    rows.salt,
                    distance,
                    (Dimension::LENGTH, rows.rule),
                    |distance| {
                        let (distance, negative) =
                            reversing::unsigned(model.parameters(), distance);
                        let solid =
                            with_extent_of(extrude, rebuild(ExtrudeEnd::Distance(distance)));
                        reversing::flipping_solid(solid, negative)
                    },
                );
                if self.reaches_far_past_the_body(extrude, distance) {
                    self.through_all_offer(ui, extrude, rebuild);
                }
            }
            ExtrudeEnd::UpToFace { target, .. } => {
                let row = PlaneRow {
                    caption: rows.face,
                    current: Some(target),
                    picker,
                    remove: None,
                };
                self.plane_row(ui, row, |plane| {
                    with_extent_of(
                        extrude,
                        rebuild(ExtrudeEnd::up_to_face(plane).with_offset(kept_offset.clone())),
                    )
                });
                self.offset_row(ui, rows, extrude, end, rebuild);
                return;
            }
            ExtrudeEnd::UpToSurface { face } => {
                let shown = Shown::Named(capitalized(&describe_origin(
                    self.document(),
                    face.face.origin(),
                )));
                feature_fields::reference_row(
                    ui,
                    self.model,
                    rows.face,
                    shown,
                    picker,
                    None,
                    self.actions,
                );
                return;
            }
            ExtrudeEnd::UpToNext { .. } => self.offset_row(ui, rows, extrude, end, rebuild),
            ExtrudeEnd::ThroughAll => {}
        }
        if self.picking(ui, slot) {
            let row = PlaneRow {
                caption: rows.face,
                current: None,
                picker,
                remove: None,
            };
            self.plane_row(ui, row, |plane| {
                with_extent_of(
                    extrude,
                    rebuild(ExtrudeEnd::up_to_face(plane).with_offset(kept_offset.clone())),
                )
            });
        }
    }

    fn reaches_far_past_the_body(&self, extrude: &Extrude, distance: &Expression) -> bool {
        let target = match extrude.operation {
            BodyOperation::Remove(target) | BodyOperation::Intersect(target) => target,
            BodyOperation::NewBody | BodyOperation::Add(_) => return false,
        };
        let parameters = self.model.shown_parameters(self.id());
        let Ok(reach) = distance.evaluate_as(Dimension::LENGTH, &|id| parameters.value(id)) else {
            return false;
        };
        let evaluation = self.model.evaluation();
        let size = std::iter::once(target)
            .chain(extrude.other_bodies.iter().copied())
            .filter_map(|body| evaluation.body_seen_by(self.id(), body)?.bounding_box())
            .map(|bounds| 2.0 * bounds.bounding_radius())
            .fold(0.0, f64::max);
        size > 0.0 && reach > FAR_PAST_THE_BODY_TIMES * size
    }

    fn through_all_offer(
        &mut self,
        ui: &mut Ui,
        extrude: &Extrude,
        rebuild: &dyn Fn(ExtrudeEnd) -> ExtrudeExtent,
    ) {
        ui.label("");
        let through = widgets::callout(ui, widgets::Tone::Info, |ui| {
            ui.add(egui::Label::new(FAR_PAST_THE_BODY).wrap());
            ui.add(widgets::button(CUT_THROUGH_ALL)).clicked()
        });
        ui.end_row();
        if through {
            let change = self.change(with_extent_of(extrude, rebuild(ExtrudeEnd::ThroughAll)));
            self.actions
                .push(feature_fields::applied(&self.feature.name, change));
        }
    }

    fn offset_row(
        &mut self,
        ui: &mut Ui,
        rows: &EndRows<'_>,
        extrude: &Extrude,
        end: &ExtrudeEnd,
        rebuild: &dyn Fn(ExtrudeEnd) -> ExtrudeExtent,
    ) {
        let offset = end
            .offset()
            .cloned()
            .unwrap_or_else(|| self.model.length_unit().default_length(0.0));
        let salt = format!("{}-offset", rows.salt);
        self.expression(
            ui,
            rows.offset,
            &salt,
            &offset,
            (Dimension::LENGTH, Rule::Any),
            |value| {
                let kept = (!is_zero(&value)).then_some(value);
                with_extent_of(extrude, rebuild(end.clone().with_offset(kept)))
            },
        );
    }

    fn extent_row(&mut self, ui: &mut Ui, extrude: &Extrude) {
        let fallback = self.default_distance();
        let current = extent_name(&extrude.extent);
        let segments = [Shape::OneSide, Shape::Symmetric, Shape::TwoSides]
            .into_iter()
            .map(|shape| {
                let extent = reshaped(&extrude.extent, shape, &fallback, &|expression| {
                    field::copied_value(self.document(), self.id(), expression)
                });
                let label = extent_name(&extent);
                Segment {
                    label,
                    hover: shape.description(),
                    change: (label != current)
                        .then(|| self.change(with_extent_of(extrude, extent))),
                }
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, "Extent", &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn extrude_rows(&mut self, ui: &mut Ui, extrude: &Extrude) {
        match &extrude.extent {
            ExtrudeExtent::OneSide { end, reversed } => {
                let reversed = *reversed;
                let rows = EndRows {
                    end: "End",
                    distance: DISTANCE,
                    face: "Up to",
                    offset: END_OFFSET,
                    salt: "distance",
                    side: Side::One,
                    rule: Rule::FlipWhenNegative,
                };
                self.end_rows(ui, &rows, extrude, end, &|end| ExtrudeExtent::OneSide {
                    end,
                    reversed,
                });
                if let Some(flipped) = feature_fields::reverse_row(ui, REVERSE_DIRECTION, reversed)
                {
                    let flipped = with_extent_of(
                        extrude,
                        ExtrudeExtent::OneSide {
                            end: end.clone(),
                            reversed: flipped,
                        },
                    );
                    self.apply(flipped);
                }
            }
            ExtrudeExtent::Symmetric { distance } => {
                self.expression(
                    ui,
                    TOTAL_DISTANCE,
                    "distance",
                    distance,
                    (Dimension::LENGTH, Rule::AboveZero),
                    |distance| {
                        SolidFeature::Extrude(Extrude {
                            extent: ExtrudeExtent::Symmetric { distance },
                            ..extrude.clone()
                        })
                    },
                );
            }
            ExtrudeExtent::TwoSides { forward, backward } => {
                let forward_rows = EndRows {
                    end: "Forward end",
                    distance: FORWARD_DISTANCE,
                    face: "Forward up to",
                    offset: FORWARD_END_OFFSET,
                    salt: "forward",
                    side: Side::Forward,
                    rule: Rule::AboveZero,
                };
                self.end_rows(ui, &forward_rows, extrude, forward, &|end| {
                    ExtrudeExtent::TwoSides {
                        forward: end,
                        backward: backward.clone(),
                    }
                });
                let backward_rows = EndRows {
                    end: "Backward end",
                    distance: BACKWARD_DISTANCE,
                    face: "Backward up to",
                    offset: BACKWARD_END_OFFSET,
                    salt: "backward",
                    side: Side::Backward,
                    rule: Rule::AboveZero,
                };
                self.end_rows(ui, &backward_rows, extrude, backward, &|end| {
                    ExtrudeExtent::TwoSides {
                        forward: forward.clone(),
                        backward: end,
                    }
                });
            }
        }
        self.direction_rows(ui, extrude);
        self.start_rows(ui, extrude.start.as_ref());
    }

    fn direction_rows(&mut self, ui: &mut Ui, extrude: &Extrude) {
        let id = self.id();
        let along = extrude.direction.as_deref();
        widgets::caption(ui, "Direction");
        self.combo(
            ui,
            "direction",
            if along.is_some() { ALONG } else { SQUARE },
            |panel| {
                let square = Choice {
                    label: SQUARE.to_owned(),
                    selected: along.is_none(),
                    change: panel
                        .change(SolidFeature::Extrude(Extrude {
                            direction: None,
                            ..extrude.clone()
                        }))
                        .map(Action::Apply),
                };
                let chosen = Choice {
                    label: ALONG.to_owned(),
                    selected: along.is_some(),
                    change: match direction_change(panel.model, panel.selection, id, extrude) {
                        Ok(transaction) => Ok(Action::Apply(transaction)),
                        Err(_) => Ok(Action::Editing(EditingCommand::Pick(Picking::new(
                            id,
                            Slot::ExtrudeDirection,
                        )))),
                    },
                };
                vec![square, chosen]
            },
        );
        ui.end_row();
        let shown = match along {
            Some(axis) => Shown::Named(capitalized(&describe_axis(self.document(), axis))),
            None if self.picking(ui, Slot::ExtrudeDirection) => Shown::NoneChosen,
            None => return,
        };
        let picker = Picker {
            feature: id,
            slot: Slot::ExtrudeDirection,
            selected: feature_fields::offered_change(
                ui.ctx(),
                self.model,
                self.selection,
                (id, Slot::ExtrudeDirection),
                || direction_change(self.model, self.selection, id, extrude),
            ),
            hover: "Run along the selected edge, axis or line instead",
        };
        let removed = feature_fields::reference_row(
            ui,
            self.model,
            "Along",
            shown,
            picker,
            along.map(|_| "Run square to the sketch again"),
            self.actions,
        );
        if removed {
            self.apply(SolidFeature::Extrude(Extrude {
                direction: None,
                ..extrude.clone()
            }));
        }
    }

    fn taper_row(&mut self, ui: &mut Ui, extrude: &Extrude) {
        let angle = extrude
            .taper
            .as_deref()
            .cloned()
            .unwrap_or_else(|| solid_tools::degrees(0.0));
        self.expression(
            ui,
            TAPER,
            "taper",
            &angle,
            (Dimension::ANGLE, Rule::Taper),
            |value| {
                SolidFeature::Extrude(Extrude {
                    taper: (!is_zero(&value)).then(|| Box::new(value)),
                    ..extrude.clone()
                })
            },
        );
    }

    fn wall_note_row(&mut self, ui: &mut Ui) {
        widgets::caption(ui, "Regions");
        ui.add(egui::Label::new(widgets::muted(WALL_CURVES, ui)).wrap());
        ui.end_row();
    }

    fn fill_rows(&mut self, ui: &mut Ui) {
        let solid = self.solid;
        let wall = solid.wall();
        let thin = solid_tools::default_wall(self.model.length_unit());
        let segments = vec![
            Segment {
                label: "Solid",
                hover: FILL_SOLID,
                change: wall
                    .is_some()
                    .then(|| self.change(solid_tools::with_wall(solid, None))),
            },
            Segment {
                label: THIN_WALL,
                hover: FILL_WALL,
                change: wall
                    .is_none()
                    .then(|| self.change(solid_tools::with_wall(solid, Some(thin)))),
            },
        ];
        let chosen = feature_fields::segmented_row(ui, "Fill", &self.feature.name, segments);
        self.actions.extend(chosen);
        let Some(wall) = wall else {
            return;
        };
        self.expression(
            ui,
            WALL_THICKNESS,
            "wall-thickness",
            &wall.thickness,
            (Dimension::LENGTH, Rule::AboveZero),
            |thickness| {
                solid_tools::with_wall(
                    solid,
                    Some(Wall {
                        thickness,
                        side: wall.side,
                    }),
                )
            },
        );
        let segments = [
            (WallSide::Inside, "Inside", WALL_INSIDE),
            (WallSide::Outside, "Outside", WALL_OUTSIDE),
            (WallSide::Centred, "Centred", WALL_CENTRED),
        ]
        .into_iter()
        .map(|(side, label, hover)| Segment {
            label,
            hover,
            change: (side != wall.side).then(|| {
                self.change(solid_tools::with_wall(
                    solid,
                    Some(Wall {
                        thickness: wall.thickness.clone(),
                        side,
                    }),
                ))
            }),
        })
        .collect();
        let chosen = feature_fields::segmented_row(ui, "Wall", &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn degrees_of(&self, angle: &Expression) -> Option<f64> {
        self.model
            .parameters()
            .evaluate_expression(angle)
            .ok()
            .map(|value| value.value)
    }

    fn two_angles(&self, angle: &Expression) -> RevolveExtent {
        let fits = self
            .degrees_of(angle)
            .is_some_and(|value| value + DEFAULT_BACKWARD_ANGLE <= FULL_TURN_DEGREES);
        RevolveExtent::TwoSides {
            forward: if fits {
                angle.clone()
            } else {
                solid_tools::degrees(DEFAULT_PARTIAL_ANGLE)
            },
            backward: solid_tools::degrees(DEFAULT_BACKWARD_ANGLE),
        }
    }

    fn turn_row(&mut self, ui: &mut Ui, revolve: &Revolve) {
        widgets::caption(ui, "Extent");
        let current = turn_name(&revolve.extent);
        self.combo(ui, "revolve-extent", current, |panel| {
            let angle = match &revolve.extent {
                RevolveExtent::Full | RevolveExtent::UpTo { .. } => {
                    solid_tools::degrees(DEFAULT_PARTIAL_ANGLE)
                }
                RevolveExtent::OneSide { angle, .. }
                | RevolveExtent::Symmetric { angle }
                | RevolveExtent::TwoSides { forward: angle, .. } => angle.clone(),
            };
            let two_angles = match &revolve.extent {
                RevolveExtent::TwoSides { .. } => revolve.extent.clone(),
                RevolveExtent::Full
                | RevolveExtent::OneSide { .. }
                | RevolveExtent::Symmetric { .. }
                | RevolveExtent::UpTo { .. } => panel.two_angles(&angle),
            };
            let mut choices: Vec<Choice> = [
                RevolveExtent::Full,
                RevolveExtent::OneSide {
                    angle: angle.clone(),
                    reversed: false,
                },
                RevolveExtent::Symmetric { angle },
                two_angles,
            ]
            .into_iter()
            .map(|extent| Choice {
                label: turn_name(&extent).to_owned(),
                selected: turn_name(&extent) == current,
                change: panel
                    .change(SolidFeature::Revolve(Revolve {
                        extent,
                        ..revolve.clone()
                    }))
                    .map(Action::Apply),
            })
            .collect();
            let up_to =
                match revolve_target_change(panel.model, panel.selection, panel.id(), revolve) {
                    Ok(transaction) => Ok(Action::Apply(transaction)),
                    Err(_) => Ok(Action::Editing(EditingCommand::Pick(Picking::new(
                        panel.id(),
                        Slot::RevolveTarget,
                    )))),
                };
            choices.push(Choice {
                label: TURN_UP_TO.to_owned(),
                selected: current == TURN_UP_TO,
                change: up_to,
            });
            choices
        });
        ui.end_row();
    }

    fn start_rows(&mut self, ui: &mut Ui, start: Option<&SolidStart>) {
        let solid = self.solid;
        let on_face = start.and_then(SolidStart::target);
        widgets::caption(ui, "Start");
        self.combo(
            ui,
            "start-kind",
            if on_face.is_some() {
                START_ON_FACE
            } else {
                START_BY_OFFSET
            },
            |panel| {
                let kept = start.filter(|start| start.distance().is_some()).cloned();
                let offset = Choice {
                    label: START_BY_OFFSET.to_owned(),
                    selected: on_face.is_none(),
                    change: panel
                        .change(solid_tools::with_start(solid, kept))
                        .map(Action::Apply),
                };
                let face = Choice {
                    label: START_ON_FACE.to_owned(),
                    selected: on_face.is_some(),
                    change: match start_change(panel.model, panel.selection, panel.id(), solid) {
                        Ok(transaction) => Ok(Action::Apply(transaction)),
                        Err(_) => Ok(Action::Editing(EditingCommand::Pick(Picking::new(
                            panel.id(),
                            Slot::StartPlane,
                        )))),
                    },
                };
                vec![offset, face]
            },
        );
        ui.end_row();
        let picker = Picker {
            feature: self.id(),
            slot: Slot::StartPlane,
            selected: feature_fields::offered_change(
                ui.ctx(),
                self.model,
                self.selection,
                (self.id(), Slot::StartPlane),
                || start_change(self.model, self.selection, self.id(), solid),
            ),
            hover: "Start from the selected flat face or plane instead",
        };
        if on_face.is_some() || self.picking(ui, Slot::StartPlane) {
            let row = PlaneRow {
                caption: "Starts at",
                current: on_face,
                picker,
                remove: on_face.map(|_| "Start at the sketch plane again"),
            };
            let removed = self.plane_row(ui, row, |plane| {
                solid_tools::with_start(solid, Some(SolidStart::Plane(plane)))
            });
            if removed {
                self.apply(solid_tools::with_start(solid, None));
            }
            if on_face.is_some() {
                return;
            }
        }
        let distance = start
            .and_then(SolidStart::distance)
            .cloned()
            .unwrap_or_else(|| self.model.length_unit().default_length(0.0));
        self.expression(
            ui,
            START_OFFSET,
            "start",
            &distance,
            (Dimension::LENGTH, Rule::Any),
            |value| {
                solid_tools::with_start(
                    solid,
                    (!is_zero(&value)).then_some(SolidStart::Distance(value)),
                )
            },
        );
    }

    fn axis_row(&mut self, ui: &mut Ui, revolve: &Revolve) {
        let model = self.model;
        let document = model.document();
        let id = self.id();
        let sketch = document
            .feature(revolve.sketch)
            .and_then(|feature| feature.kind.sketch());
        let row = AxisRow {
            caption: AXIS,
            shown: Some(solid_tools::axis_name(
                document,
                revolve.sketch,
                &revolve.axis,
            )),
            picker: Picker {
                feature: id,
                slot: Slot::RevolveAxis,
                selected: feature_fields::offered_change(
                    ui.ctx(),
                    model,
                    self.selection,
                    (id, Slot::RevolveAxis),
                    || selected_axis_change(model, self.selection, id, revolve),
                ),
                hover: "Turn about the selected axis, edge or round face",
            },
        };
        let turned_about = |axis| {
            self.change(SolidFeature::Revolve(Revolve {
                axis,
                ..revolve.clone()
            }))
        };
        let listed = || {
            let lines = sketch.into_iter().flat_map(|sketch| {
                let lines = sketch
                    .entities()
                    .filter_map(|(id, entity)| matches!(entity, Entity::Line { .. }).then_some(id));
                [Reference::HorizontalAxis.id(), Reference::VerticalAxis.id()]
                    .into_iter()
                    .chain(lines)
                    .map(|line| {
                        let axis = RevolveAxis::Sketch(line);
                        let selected = revolve.axis == axis;
                        Choice {
                            label: sketch.entity_label(line),
                            selected,
                            change: if selected {
                                Err(feature_fields::ALREADY_CHOSEN.to_owned())
                            } else {
                                turned_about(axis).map(Action::Apply)
                            },
                        }
                    })
            });
            let model_axes =
                feature_fields::model_axis_choices(model, id, revolve.axis.model(), &|axis| {
                    match off_sketch_plane(model, (id, revolve.sketch), &axis) {
                        Some(reason) => Err(reason),
                        None => turned_about(RevolveAxis::Model(axis)),
                    }
                });
            lines.chain(model_axes).collect()
        };
        let mut actions = Vec::new();
        feature_fields::axis_row(ui, model, row, Some(&listed), &mut actions);
        self.actions.extend(actions);
    }

    fn side_rows(&mut self, ui: &mut Ui, revolve: &Revolve) {
        let with_side = |side| {
            SolidFeature::Revolve(Revolve {
                side,
                ..revolve.clone()
            })
        };
        let one_side = revolve
            .side
            .or_else(|| larger_side(self.model, self.id(), revolve))
            .unwrap_or(AxisSide::Left);
        let segments = vec![
            Segment {
                label: "Whole",
                hover: WHOLE_PROFILE,
                change: revolve.side.is_some().then(|| self.change(with_side(None))),
            },
            Segment {
                label: "One side",
                hover: ONE_SIDE_OF_AXIS,
                change: revolve
                    .side
                    .is_none()
                    .then(|| self.change(with_side(Some(one_side)))),
            },
        ];
        let chosen = feature_fields::segmented_row(ui, "Profile", &self.feature.name, segments);
        self.actions.extend(chosen);
        if let Some(side) = revolve.side
            && feature_fields::reverse_row(ui, KEEP_OTHER_SIDE, side == AxisSide::Right).is_some()
        {
            self.apply(with_side(Some(side.other())));
        }
    }

    fn revolve_rows(&mut self, ui: &mut Ui, revolve: &Revolve) {
        self.axis_row(ui, revolve);
        if revolve.wall.is_none() {
            self.side_rows(ui, revolve);
        }
        if revolve.extent.target().is_none() {
            self.revolve_target_row(ui, revolve, None);
        }
        match &revolve.extent {
            RevolveExtent::Full => {}
            RevolveExtent::UpTo { target, reversed } => {
                self.revolve_target_row(ui, revolve, Some(target));
                if let Some(flipped) = feature_fields::reverse_row(ui, REVERSE_DIRECTION, *reversed)
                {
                    let flipped = SolidFeature::Revolve(Revolve {
                        extent: RevolveExtent::up_to((**target).clone(), flipped),
                        ..revolve.clone()
                    });
                    self.apply(flipped);
                }
            }
            RevolveExtent::OneSide { angle, reversed } => {
                let reversed = *reversed;
                self.expression(
                    ui,
                    ANGLE,
                    "angle",
                    angle,
                    (Dimension::ANGLE, Rule::Turn),
                    |angle| {
                        SolidFeature::Revolve(Revolve {
                            extent: RevolveExtent::OneSide { angle, reversed },
                            ..revolve.clone()
                        })
                    },
                );
                if let Some(flipped) = feature_fields::reverse_row(ui, REVERSE_DIRECTION, reversed)
                {
                    let flipped = SolidFeature::Revolve(Revolve {
                        extent: RevolveExtent::OneSide {
                            angle: angle.clone(),
                            reversed: flipped,
                        },
                        ..revolve.clone()
                    });
                    self.apply(flipped);
                }
            }
            RevolveExtent::Symmetric { angle } => {
                self.expression(
                    ui,
                    TOTAL_ANGLE,
                    "angle",
                    angle,
                    (Dimension::ANGLE, Rule::Turn),
                    |angle| {
                        SolidFeature::Revolve(Revolve {
                            extent: RevolveExtent::Symmetric { angle },
                            ..revolve.clone()
                        })
                    },
                );
            }
            RevolveExtent::TwoSides { forward, backward } => {
                let beside =
                    |other: &Expression| Rule::TurnBeside(self.degrees_of(other).unwrap_or(0.0));
                let (forward_rule, backward_rule) = (beside(backward), beside(forward));
                self.expression(
                    ui,
                    FORWARD_ANGLE,
                    "forward-angle",
                    forward,
                    (Dimension::ANGLE, forward_rule),
                    |forward| {
                        SolidFeature::Revolve(Revolve {
                            extent: RevolveExtent::TwoSides {
                                forward,
                                backward: backward.clone(),
                            },
                            ..revolve.clone()
                        })
                    },
                );
                self.expression(
                    ui,
                    BACKWARD_ANGLE,
                    "backward-angle",
                    backward,
                    (Dimension::ANGLE, backward_rule),
                    |backward| {
                        SolidFeature::Revolve(Revolve {
                            extent: RevolveExtent::TwoSides {
                                forward: forward.clone(),
                                backward,
                            },
                            ..revolve.clone()
                        })
                    },
                );
            }
        }
        self.start_rows(ui, revolve.start.as_ref());
    }

    fn revolve_target_row(
        &mut self,
        ui: &mut Ui,
        revolve: &Revolve,
        target: Option<&PlaneReference>,
    ) {
        let id = self.id();
        if target.is_none() && !self.picking(ui, Slot::RevolveTarget) {
            return;
        }
        let picker = Picker {
            feature: id,
            slot: Slot::RevolveTarget,
            selected: feature_fields::offered_change(
                ui.ctx(),
                self.model,
                self.selection,
                (id, Slot::RevolveTarget),
                || revolve_target_change(self.model, self.selection, id, revolve),
            ),
            hover: "Turn up to the selected face or plane through the axis instead",
        };
        let reversed = matches!(revolve.extent, RevolveExtent::UpTo { reversed: true, .. });
        let row = PlaneRow {
            caption: "Up to",
            current: target,
            picker,
            remove: None,
        };
        self.plane_row(ui, row, |plane| {
            SolidFeature::Revolve(Revolve {
                extent: RevolveExtent::up_to(plane, reversed),
                ..revolve.clone()
            })
        });
    }

    fn operation_rows(&mut self, ui: &mut Ui) {
        let operation = self.solid.operation();
        let bodies = self.document().bodies_before(self.id());
        let fallback = bodies.last().copied();
        widgets::caption(ui, "Result");
        self.combo(ui, "operation", operation_name(operation), |panel| {
            let target = operation.target().or(fallback);
            let candidates = [
                Some(BodyOperation::NewBody),
                target.map(BodyOperation::Add),
                target.map(BodyOperation::Remove),
                target.map(BodyOperation::Intersect),
            ];
            candidates
                .into_iter()
                .flatten()
                .map(|candidate| Choice {
                    label: operation_name(candidate).to_owned(),
                    selected: operation_name(candidate) == operation_name(operation),
                    change: panel
                        .change(solid_tools::with_operation(panel.solid, candidate))
                        .map(Action::Apply),
                })
                .collect()
        });
        ui.end_row();
        let Some(target) = operation.target() else {
            return;
        };
        widgets::caption(ui, "Body");
        let model = self.model;
        ui.horizontal(|ui| {
            let name = feature_fields::feature_name(model.document(), target);
            let text = feature_fields::combo_text(ui, name, MISSING_BODY);
            self.combo(ui, "body", text, |panel| {
                bodies
                    .iter()
                    .filter_map(|body| {
                        let label = panel.document().feature(*body)?.name.clone();
                        Some(Choice {
                            label,
                            selected: *body == target,
                            change: panel
                                .change(solid_tools::with_operation(
                                    panel.solid,
                                    operation.with_target(*body),
                                ))
                                .map(Action::Apply),
                        })
                    })
                    .collect()
            });
        });
        ui.end_row();
        if let BodyOperation::Remove(_) = operation {
            self.other_body_rows(ui, &bodies, target);
        }
    }

    fn other_body_rows(&mut self, ui: &mut Ui, bodies: &[FeatureId], target: FeatureId) {
        let others = self.solid.other_bodies().to_vec();
        for (index, other) in others.iter().enumerate() {
            if index == 0 {
                widgets::caption(ui, ALSO_CUTS);
            } else {
                ui.label("");
            }
            let name = feature_fields::feature_name(self.document(), *other);
            let hover = format!("Stop cutting {}", name.unwrap_or("the missing body"));
            let mut dropped = false;
            ui.horizontal(|ui| {
                match &name {
                    Some(name) => {
                        ui.label(*name);
                    }
                    None => feature_fields::missing(ui, MISSING_BODY),
                }
                dropped = widgets::icon_button(ui, icons::REMOVE, &hover).clicked();
            });
            ui.end_row();
            if dropped {
                let kept = others
                    .iter()
                    .copied()
                    .filter(|kept| kept != other)
                    .collect();
                self.apply(solid_tools::with_other_bodies(self.solid, kept));
            }
        }
        let candidates: Vec<FeatureId> = bodies
            .iter()
            .copied()
            .filter(|body| *body != target && !others.contains(body))
            .collect();
        if candidates.is_empty() {
            return;
        }
        if others.is_empty() {
            widgets::caption(ui, ALSO_CUTS);
        } else {
            ui.label("");
        }
        self.combo(ui, "also-cut", ADD_CUT_BODY, |panel| {
            candidates
                .iter()
                .filter_map(|body| {
                    let label = panel.document().feature(*body)?.name.clone();
                    let mut chosen = others.clone();
                    chosen.push(*body);
                    Some(Choice {
                        label,
                        selected: false,
                        change: panel
                            .change(solid_tools::with_other_bodies(panel.solid, chosen))
                            .map(Action::Apply),
                    })
                })
                .collect()
        });
        ui.end_row();
    }

    fn path_row(&mut self, ui: &mut Ui, sweep: &Sweep) {
        let id = self.id();
        let shown = if sweep.path.is_empty() {
            Shown::NoneChosen
        } else {
            Shown::Named(sweep_tools::path_words(self.document(), &sweep.path))
        };
        let picker = Picker {
            feature: id,
            slot: Slot::SweepPath,
            selected: feature_fields::offered_change(
                ui.ctx(),
                self.model,
                self.selection,
                (id, Slot::SweepPath),
                || sweep_tools::selected_change(self.model, self.selection, id, sweep),
            ),
            hover: FOLLOW_SELECTED_PATH,
        };
        let removed = feature_fields::reference_row(
            ui,
            self.model,
            PATH,
            shown,
            picker,
            (!sweep.path.is_empty()).then_some(CLEAR_PATH),
            self.actions,
        );
        if removed {
            self.apply(SolidFeature::Sweep(Sweep {
                path: SweepPath::Unchosen,
                ..sweep.clone()
            }));
        }
    }

    fn orientation_row(&mut self, ui: &mut Ui, sweep: &Sweep) {
        let with = |orientation| {
            (sweep.orientation != orientation).then(|| {
                self.change(SolidFeature::Sweep(Sweep {
                    orientation,
                    ..sweep.clone()
                }))
            })
        };
        let segments = vec![
            Segment {
                label: SQUARE_TO_PATH,
                hover: SQUARE_TO_PATH_HOVER,
                change: with(SweepOrientation::Square),
            },
            Segment {
                label: PARALLEL,
                hover: PARALLEL_HOVER,
                change: with(SweepOrientation::Parallel),
            },
        ];
        let chosen = feature_fields::segmented_row(ui, ORIENTATION, &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn apply(&mut self, solid: SolidFeature) {
        let change = self.change(solid);
        self.actions
            .push(feature_fields::applied(&self.feature.name, change));
    }
}

pub fn is_zero(expression: &Expression) -> bool {
    matches!(
        expression,
        Expression::Number(value) | Expression::Measure(value, _) if *value == 0.0
    )
}

fn with_extent_of(extrude: &Extrude, extent: ExtrudeExtent) -> SolidFeature {
    SolidFeature::Extrude(Extrude {
        extent,
        ..extrude.clone()
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    OneSide,
    Symmetric,
    TwoSides,
}

impl Shape {
    fn description(self) -> &'static str {
        match self {
            Self::OneSide => "Extrude from the sketch to one side",
            Self::Symmetric => "Extrude the same distance to both sides of the sketch",
            Self::TwoSides => "Extrude to each side by its own end",
        }
    }
}

fn reshaped(
    extent: &ExtrudeExtent,
    shape: Shape,
    fallback: &Expression,
    copied: &dyn Fn(&Expression) -> Expression,
) -> ExtrudeExtent {
    let distance = extent
        .ends()
        .into_iter()
        .find_map(ExtrudeEnd::distance)
        .or(match extent {
            ExtrudeExtent::Symmetric { distance } => Some(distance),
            ExtrudeExtent::OneSide { .. } | ExtrudeExtent::TwoSides { .. } => None,
        })
        .unwrap_or(fallback)
        .clone();
    match (extent, shape) {
        (ExtrudeExtent::OneSide { .. }, Shape::OneSide)
        | (ExtrudeExtent::Symmetric { .. }, Shape::Symmetric)
        | (ExtrudeExtent::TwoSides { .. }, Shape::TwoSides) => extent.clone(),
        (ExtrudeExtent::TwoSides { forward: end, .. }, Shape::OneSide) => ExtrudeExtent::OneSide {
            end: end.clone(),
            reversed: false,
        },
        (ExtrudeExtent::Symmetric { .. }, Shape::OneSide) => {
            ExtrudeExtent::one_side(distance, false)
        }
        (ExtrudeExtent::OneSide { .. } | ExtrudeExtent::TwoSides { .. }, Shape::Symmetric) => {
            ExtrudeExtent::Symmetric { distance }
        }
        (ExtrudeExtent::OneSide { end, reversed }, Shape::TwoSides) => {
            let other = ExtrudeEnd::Distance(copied(&distance));
            if *reversed {
                ExtrudeExtent::TwoSides {
                    forward: other,
                    backward: end.clone(),
                }
            } else {
                ExtrudeExtent::TwoSides {
                    forward: end.clone(),
                    backward: other,
                }
            }
        }
        (ExtrudeExtent::Symmetric { .. }, Shape::TwoSides) => {
            let backward = copied(&distance);
            ExtrudeExtent::two_sides(distance, backward)
        }
    }
}

fn not_a_target(model: &Model, pickable: Pickable, index: usize) -> Option<&'static str> {
    match pickable {
        Pickable::Face { body, face } => {
            match sketch_placement::attachment_at(model, FaceChoice { body, face }, index) {
                Err(sketch_placement::NOT_FLAT) => Some(CURVED_TARGET),
                Err(reason) => Some(reason),
                Ok(_) => None,
            }
        }
        Pickable::Datum(datum) if datum_tools::is_plane(model.document(), datum) => {
            Some("The selected plane comes after this feature in the tree")
        }
        Pickable::FramePlane { .. } => {
            Some("The selected coordinate system comes after this feature in the tree")
        }
        Pickable::Datum(_) => Some("The selected datum is not a plane"),
        _ => None,
    }
}

pub fn selected_target(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
) -> Result<PlaneReference, String> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or_else(|| "The feature no longer exists".to_owned())?;
    let mut targets = Vec::new();
    let mut refused = None;
    for pickable in selection.iter() {
        match datum_tools::plane_reference(model, pickable, index) {
            Some(target) => targets.push(target),
            None => {
                refused = refused.or_else(|| not_a_target(model, pickable, index));
            }
        }
    }
    match targets.as_slice() {
        [target] => Ok(target.clone()),
        [] => Err(refused.unwrap_or(NO_TARGET_SELECTED).to_owned()),
        _ => Err("Select only one face or plane".to_owned()),
    }
}

fn selected_end(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    offset: Option<Expression>,
) -> Result<ExtrudeEnd, String> {
    let flat = match selected_target(model, selection, feature) {
        Ok(target) => return Ok(ExtrudeEnd::up_to_face(target).with_offset(offset)),
        Err(reason) => reason,
    };
    let picked: Vec<Pickable> = selection.iter().collect();
    let [Pickable::Face { body, face }] = picked.as_slice() else {
        return Err(flat);
    };
    let index = model
        .document()
        .feature_index(feature)
        .ok_or_else(|| "The feature no longer exists".to_owned())?;
    let choice = FaceChoice {
        body: *body,
        face: *face,
    };
    sketch_placement::surface_at(model, choice, index)
        .map(ExtrudeEnd::up_to_surface)
        .map_err(str::to_owned)
}

fn end_on(extent: &ExtrudeExtent, side: Side) -> Option<&ExtrudeEnd> {
    match (extent, side) {
        (ExtrudeExtent::OneSide { end, .. }, Side::One) => Some(end),
        (ExtrudeExtent::TwoSides { forward, .. }, Side::Forward) => Some(forward),
        (ExtrudeExtent::TwoSides { backward, .. }, Side::Backward) => Some(backward),
        _ => None,
    }
}

fn with_end_on(extent: &ExtrudeExtent, side: Side, end: ExtrudeEnd) -> Option<ExtrudeExtent> {
    match (extent, side) {
        (ExtrudeExtent::OneSide { reversed, .. }, Side::One) => Some(ExtrudeExtent::OneSide {
            end,
            reversed: *reversed,
        }),
        (ExtrudeExtent::TwoSides { backward, .. }, Side::Forward) => {
            Some(ExtrudeExtent::TwoSides {
                forward: end,
                backward: backward.clone(),
            })
        }
        (ExtrudeExtent::TwoSides { forward, .. }, Side::Backward) => {
            Some(ExtrudeExtent::TwoSides {
                forward: forward.clone(),
                backward: end,
            })
        }
        _ => None,
    }
}

pub fn target_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    extrude: &Extrude,
    side: Side,
) -> Result<Transaction, String> {
    let kept = end_on(&extrude.extent, side)
        .and_then(ExtrudeEnd::offset)
        .cloned();
    let end = selected_end(model, selection, feature, kept)?;
    let extent = with_end_on(&extrude.extent, side, end).ok_or_else(|| SIDES_CHANGED.to_owned())?;
    if extent == extrude.extent {
        return Err("This end already runs up to the selected face or plane".to_owned());
    }
    change(model, feature, with_extent_of(extrude, extent))
}

pub fn up_to_selected_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    extrude: &Extrude,
) -> Result<Transaction, String> {
    let kept = extrude
        .extent
        .ends()
        .first()
        .and_then(|end| end.offset())
        .cloned();
    let end = selected_end(model, selection, feature, kept)?;
    let extent = match &extrude.extent {
        ExtrudeExtent::OneSide { reversed, .. } => ExtrudeExtent::OneSide {
            end,
            reversed: *reversed,
        },
        ExtrudeExtent::Symmetric { .. } => ExtrudeExtent::OneSide {
            end,
            reversed: false,
        },
        ExtrudeExtent::TwoSides { backward, .. } => ExtrudeExtent::TwoSides {
            forward: end,
            backward: backward.clone(),
        },
    };
    if extent == extrude.extent {
        return Err("The extrusion already runs up to the selected face or plane".to_owned());
    }
    change(model, feature, with_extent_of(extrude, extent))
}

pub fn direction_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    extrude: &Extrude,
) -> Result<Transaction, String> {
    let index = model.document().feature_index(feature).unwrap_or(0);
    let axis = datum_tools::only_axis(model, selection, index)?
        .ok_or_else(|| NO_DIRECTION_SELECTED.to_owned())?;
    if extrude.direction.as_deref() == Some(&axis) {
        return Err("The extrusion already runs along the selected edge or axis".to_owned());
    }
    change(
        model,
        feature,
        SolidFeature::Extrude(Extrude {
            direction: Some(Box::new(axis)),
            ..extrude.clone()
        }),
    )
}

pub fn revolve_target_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    revolve: &Revolve,
) -> Result<Transaction, String> {
    let target = selected_target(model, selection, feature)?;
    let reversed = matches!(revolve.extent, RevolveExtent::UpTo { reversed: true, .. });
    let extent = RevolveExtent::up_to(target, reversed);
    if extent == revolve.extent {
        return Err("The revolution already turns up to the selected face or plane".to_owned());
    }
    change(
        model,
        feature,
        SolidFeature::Revolve(Revolve {
            extent,
            ..revolve.clone()
        }),
    )
}

pub fn start_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    solid: &SolidFeature,
) -> Result<Transaction, String> {
    if let SolidFeature::Sweep(_) = solid {
        return Err("A sweep starts where its profile is drawn".to_owned());
    }
    let target = selected_target(model, selection, feature)?;
    if solid.start().and_then(SolidStart::target) == Some(&target) {
        return Err("This feature already starts from the selected face or plane".to_owned());
    }
    change(
        model,
        feature,
        solid_tools::with_start(solid, Some(SolidStart::Plane(target))),
    )
}

pub fn selected_axis_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    revolve: &Revolve,
) -> Result<Transaction, String> {
    let index = model.document().feature_index(feature).unwrap_or(0);
    let chosen = datum_tools::only_axis(model, selection, index)?.map(|axis| match axis {
        AxisReference::Sketch { sketch, entity } if sketch == revolve.sketch => {
            RevolveAxis::Sketch(entity)
        }
        axis => RevolveAxis::Model(axis),
    });
    match chosen {
        Some(axis) if revolve.axis == axis => {
            Err("The revolve already turns about the selected axis".to_owned())
        }
        Some(axis) => change(
            model,
            feature,
            SolidFeature::Revolve(Revolve {
                axis,
                ..revolve.clone()
            }),
        ),
        None => Err(
            "Select an axis, a straight edge or a round face made before this feature".to_owned(),
        ),
    }
}

fn off_sketch_plane(
    model: &Model,
    (feature, sketch): (FeatureId, FeatureId),
    axis: &AxisReference,
) -> Option<String> {
    let (document, evaluation) = (model.document(), model.evaluation());
    let plane = scene::sketch_plane(document, evaluation, sketch)?;
    let ray = displayed_axis(evaluation, feature, axis)?;
    let sketch_name = feature_fields::feature_name(document, sketch).unwrap_or("the sketch");
    (!along_plane(ray, &plane)).then(|| {
        format!(
            "{} does not lie in the plane of {sketch_name}",
            capitalized(&describe_axis(document, axis))
        )
    })
}

fn larger_side(model: &Model, feature: FeatureId, revolve: &Revolve) -> Option<AxisSide> {
    let (document, evaluation) = (model.document(), model.evaluation());
    let plane = scene::sketch_plane(document, evaluation, revolve.sketch)?;
    let (origin, toward) = match &revolve.axis {
        RevolveAxis::Sketch(line) => match line.reference() {
            Some(Reference::HorizontalAxis) => (Point2::ZERO, Point2::X),
            Some(Reference::VerticalAxis) => (Point2::ZERO, Point2::Y),
            Some(Reference::Origin) => return None,
            None => evaluation
                .feature(revolve.sketch)?
                .result
                .as_deref()?
                .sketch()?
                .geometry
                .line_endpoints(*line)?,
        },
        RevolveAxis::Model(axis) => {
            let ray = displayed_axis(evaluation, feature, axis)?;
            (
                plane.to_local(ray.origin()),
                plane.to_local(ray.origin() + ray.direction()),
            )
        }
    };
    let direction = (toward - origin).try_normalize()?;
    let (_, regions) = selection::swept_regions(document, evaluation, feature)?;
    let chosen = scene::chosen_regions(&revolve.regions, regions);
    let leftward: f64 = regions
        .iter()
        .filter(|region| chosen.contains(&region.region.key()))
        .filter_map(|region| region.mesh.as_ref())
        .flat_map(|mesh| {
            mesh.triangles.iter().filter_map(|triangle| {
                let [a, b, c] = triangle.map(|index| mesh.points.get(index as usize).copied());
                let [a, b, c] = [a?, b?, c?];
                let area = 0.5 * (b - a).perp_dot(c - a).abs();
                let side = direction.perp_dot((a + b + c) / 3.0 - origin).signum();
                Some(area * side)
            })
        })
        .sum();
    Some(if leftward < 0.0 {
        AxisSide::Right
    } else {
        AxisSide::Left
    })
}

fn change(model: &Model, feature: FeatureId, solid: SolidFeature) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = solid_tools::edit(document, feature, solid)
        .ok_or_else(|| "The feature no longer exists".to_owned())?;
    field::checked(document, transaction)
}

fn with_sketch(solid: &SolidFeature, sketch: FeatureId) -> SolidFeature {
    match solid {
        SolidFeature::Extrude(extrude) => SolidFeature::Extrude(Extrude {
            sketch,
            regions: RegionChoice::All,
            ..extrude.clone()
        }),
        SolidFeature::Revolve(revolve) => SolidFeature::Revolve(Revolve {
            sketch,
            regions: RegionChoice::All,
            axis: match &revolve.axis {
                RevolveAxis::Sketch(line) if !line.is_reference() => {
                    RevolveAxis::Sketch(EntityId::VERTICAL_AXIS)
                }
                RevolveAxis::Sketch(_) | RevolveAxis::Model(_) => revolve.axis.clone(),
            },
            ..revolve.clone()
        }),
        SolidFeature::Sweep(sweep) => SolidFeature::Sweep(Sweep {
            sketch,
            regions: RegionChoice::All,
            ..sweep.clone()
        }),
    }
}

fn extent_name(extent: &ExtrudeExtent) -> &'static str {
    match extent {
        ExtrudeExtent::OneSide { .. } => "One side",
        ExtrudeExtent::Symmetric { .. } => "Symmetric",
        ExtrudeExtent::TwoSides { .. } => "Two sides",
    }
}

fn turn_name(extent: &RevolveExtent) -> &'static str {
    match extent {
        RevolveExtent::UpTo { .. } => TURN_UP_TO,
        RevolveExtent::Full => "Full turn",
        RevolveExtent::OneSide { .. } => "One side",
        RevolveExtent::Symmetric { .. } => "Symmetric",
        RevolveExtent::TwoSides { .. } => "Two angles",
    }
}

fn operation_name(operation: BodyOperation) -> &'static str {
    match operation {
        BodyOperation::NewBody => "New body",
        BodyOperation::Add(_) => "Add to body",
        BodyOperation::Remove(_) => "Remove from body",
        BodyOperation::Intersect(_) => "Intersect with body",
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    actions: &mut Vec<Action>,
    feature: &Feature,
    solid: &SolidFeature,
    opened: bool,
) {
    let mut panel = Panel {
        model,
        selection,
        feature,
        solid,
        actions,
    };
    widgets::properties(ui, ("solid-properties", feature.id()), |ui| {
        match solid {
            SolidFeature::Extrude(extrude) => panel.extent_row(ui, extrude),
            SolidFeature::Revolve(revolve) => panel.turn_row(ui, revolve),
            SolidFeature::Sweep(_) => {}
        }
        panel.sketch_row(ui);
        if solid.wall().is_some() {
            panel.wall_note_row(ui);
        } else {
            panel.regions_row(ui, opened);
        }
        match solid {
            SolidFeature::Extrude(extrude) => {
                panel.fill_rows(ui);
                panel.extrude_rows(ui, extrude);
                panel.taper_row(ui, extrude);
            }
            SolidFeature::Revolve(revolve) => {
                panel.fill_rows(ui);
                panel.revolve_rows(ui, revolve);
            }
            SolidFeature::Sweep(sweep) => {
                panel.path_row(ui, sweep);
                panel.orientation_row(ui, sweep);
            }
        }
        panel.operation_rows(ui);
    });
}
