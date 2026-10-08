mod attachment;
mod blend;
mod body_appearance;
mod combine;
mod datum;
mod dependencies;
mod describe;
mod document;
mod edit;
mod editor;
mod grouping;
mod healing;
mod history;
mod hole;
mod hole_standard;
mod import;
mod inlining;
mod mirror;
mod movement;
mod origins;
mod parameter_list;
mod pattern;
mod pieces;
mod presenting;
mod projection;
mod properties;
mod recompute;
mod removal;
mod scaling;
mod shell;
mod solid;
mod split;
mod tolerance;
mod tree;
mod trouble;
mod values;
mod views;
mod worker;

pub use crate::{
    attachment::{AttachmentError, FaceAttachment, SketchAttachment, SketchFeature, face_plane},
    blend::{Blend, BlendKind},
    body_appearance::{
        BodyAppearance, DensityError, FaceColour, MAX_BODY_NAME_CHARS, MAX_DENSITY,
        MAX_MATERIAL_NAME_CHARS, MIN_OPACITY_PERCENT, OPAQUE_PERCENT, Rgb, density_of,
        material_name,
    },
    combine::{Combine, CombineOperation},
    datum::{
        AxisReference, Datum, DatumAxis, DatumKind, DatumPlane, DatumPoint, DatumResult,
        PlaneReference, PlaneRotation, PlaneThrough, PointReference, PrincipalAxis,
        PrincipalGeometry, PrincipalPlane, capitalized, describe_axis, describe_plane,
        describe_point, describe_points, displayed_axis,
    },
    dependencies::DependencyGraph,
    describe::{describe_edge, describe_origin, edge_faces, origin_feature},
    document::{
        Document, FIRST_UNSTORABLE_ID, Feature, FeatureId, FeatureKind, Parameter, ParameterUser,
        RollbackBar, TreeRow,
    },
    edit::{Edit, EditError, MAX_PARAMETER_NOTE_CHARS, Touched, Transaction, TransactionBuilder},
    editor::{Base, Editor, Prepared, Stale},
    grouping::{MAX_GROUP_NAME_CHARS, group_name},
    healing::Healing,
    hole::{
        CircleSize, Hole, HoleDepth, HoleShape, HoleSizing, HoleStep, HoleStyle,
        MAX_COUNTERSINK_ANGLE, MAX_HOLE_STEPS, MAX_HOLES, centres as hole_centres, circle_sizes,
    },
    hole_standard::{FinePitch, HeatSetInsert, HoleFit, HoleStandard, MetricSize, pitch_text},
    import::Import,
    mirror::{MIRROR_IMAGE, Mirror},
    movement::{AxisTurn, BodyPlacement, Move, MoveAxis, Pivot, TurnCentre},
    origins::complete_origins,
    pattern::{
        CircularPattern, Instance, LinearDirection, LinearSpacing, MAX_PATTERN_INSTANCES,
        ORIGINAL_INSTANCE, Pattern, PatternKind, instance_name, repeatable_on,
    },
    pieces::{Resolution, Unresolved},
    projection::{
        Outline, PROJECTED_SPLINE_POINTS, ProjectionSource, edge_outline, sketch_outline,
        vertex_outline,
    },
    properties::{
        MAX_DESCRIPTION_CHARS, MAX_MODEL_NOTES_CHARS, MAX_PROPERTY_CHARS, ModelProperties,
        ModelProperty,
    },
    recompute::{
        CancelToken, Evaluation, Evaluator, Failure, FeatureError, FeatureResult, FeatureState,
        FeatureStatus, FixTarget, Inputs, ModelEvaluator, Recompute, SketchResult,
    },
    removal::Remove,
    scaling::{MAX_SCALE_FACTOR, MIN_SCALE_FACTOR, Scale},
    shell::Shell,
    solid::{
        AxisSide, BodyOperation, Extrude, ExtrudeEnd, ExtrudeExtent, NameIndex, RegionChoice,
        Revolve, RevolveAxis, RevolveExtent, SketchRegion, SolidFeature, SolidResult, SolidStart,
        body_part, body_parts, profile_curve, sketch_regions,
    },
    split::Split,
    values::{ParameterError, ParameterValues},
    views::{
        HOME_VIEW_NAME, MAX_SAVED_VIEWS, MAX_VIEW_NAME_CHARS, NamedView, SavedView, SavedViews,
        view_name,
    },
    worker::{Outcome, Progress, Recomputer, Update, WorkerStopped},
};

#[cfg(test)]
mod attachment_tests;
#[cfg(test)]
mod blend_tests;
#[cfg(test)]
mod body_appearance_tests;
#[cfg(test)]
mod combine_tests;
#[cfg(test)]
mod cut_several_tests;
#[cfg(test)]
mod datum_point_tests;
#[cfg(test)]
mod datum_tests;
#[cfg(test)]
mod extent_tests;
#[cfg(test)]
mod grouping_tests;
#[cfg(test)]
mod history_tests;
#[cfg(test)]
mod hole_tests;
#[cfg(test)]
mod mirror_tests;
#[cfg(test)]
mod movement_tests;
#[cfg(test)]
mod parameter_tests;
#[cfg(test)]
mod pattern_tests;
#[cfg(test)]
mod presenting_tests;
#[cfg(test)]
mod projection_tests;
#[cfg(test)]
mod properties_tests;
#[cfg(test)]
mod removal_tests;
#[cfg(test)]
mod scaling_tests;
#[cfg(test)]
mod shell_tests;
#[cfg(test)]
mod sketch_tests;
#[cfg(test)]
mod solid_tests;
#[cfg(test)]
mod split_tests;
#[cfg(test)]
mod start_tests;
#[cfg(test)]
mod tree_tests;
#[cfg(test)]
mod views_tests;
#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use caditor_expression::{EvalError, Expression, ParameterId, Quantity};
    use caditor_geometry::{Plane, Point2};
    use caditor_sketch::{Constraint, ConstraintId, Entity, EntityId, Sketch};

    use super::*;

    pub struct Ids {
        pub width: ParameterId,
        pub height: ParameterId,
        pub gap: ParameterId,
        pub base: FeatureId,
        pub side: FeatureId,
        pub base_distance: ConstraintId,
        pub side_distance: ConstraintId,
    }

    fn line_with_distance(plane: Plane, length: f64, value: Expression) -> (Sketch, ConstraintId) {
        let mut sketch = Sketch::new(plane);
        let line = sketch.add_line(Point2::ZERO, Point2::new(length, 0.0));
        let Some(Entity::Line { start, end }) = sketch.entity(line).cloned() else {
            panic!("expected a line");
        };
        sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
        let distance = sketch
            .add_constraint(Constraint::Distance {
                from: start,
                to: end,
                value,
            })
            .unwrap();
        (sketch, distance)
    }

    pub fn sample() -> (Document, Ids) {
        let mut document = Document::default();
        let mut transaction = document.transaction("Sample");
        let width = transaction.add_parameter("width", transaction.parse("40 mm").unwrap());
        let height = transaction.add_parameter("height", transaction.parse("20 mm").unwrap());
        let gap = transaction.add_parameter("gap", transaction.parse("5 mm").unwrap());
        let (base_sketch, base_distance) =
            line_with_distance(Plane::XY, 40.0, Expression::Parameter(width));
        let (side_sketch, side_distance) =
            line_with_distance(Plane::XZ, 20.0, Expression::Parameter(height));
        let base = transaction.add_feature("Base sketch", FeatureKind::from(base_sketch));
        let side = transaction.add_feature("Side sketch", FeatureKind::from(side_sketch));
        document.apply(transaction.finish()).unwrap();
        (
            document,
            Ids {
                width,
                height,
                gap,
                base,
                side,
                base_distance,
                side_distance,
            },
        )
    }

    fn set_expression(document: &Document, id: ParameterId, text: &str) -> Transaction {
        Transaction::single(
            "Edit parameter",
            Edit::SetParameterExpression {
                id,
                expression: document.parse(text).unwrap(),
            },
        )
    }

    fn recompute(recompute: &mut Recompute, document: &Document) -> Evaluation {
        recompute.run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
    }

    fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
        match &evaluation.feature(feature).unwrap().state {
            FeatureState::Failed(error) => error.clone(),
            other => panic!("expected a failure, found {other:?}"),
        }
    }

    #[test]
    fn a_whole_document_can_be_turned_into_an_earlier_one_and_back_by_undo() {
        let (earlier, ids) = sample();
        let mut later = earlier.clone();
        let mut transaction = later.transaction("Grow");
        let depth = transaction.add_parameter("depth", transaction.parse("width / 2").unwrap());
        transaction.add_feature("Top sketch", FeatureKind::from(Sketch::new(Plane::YZ)));
        later.apply(transaction.finish()).unwrap();
        later
            .apply(set_expression(&later, ids.height, "depth + 1 mm"))
            .unwrap();
        later
            .apply(Transaction::single(
                "Drop gap",
                Edit::RemoveParameter { id: ids.gap },
            ))
            .unwrap();

        let mut editor = Editor::new(later.clone());
        editor
            .apply(later.transaction_to(&earlier, "Restore"))
            .unwrap();
        assert!(editor.document().same_content(&earlier));
        assert!(editor.document().parameter(depth).is_none());
        assert_eq!(
            editor.document().next_parameter_id(),
            later.next_parameter_id()
        );

        editor.undo().unwrap();
        assert!(editor.document().same_content(&later));
    }

    #[test]
    fn hiding_principal_geometry_is_undoable_content_that_a_restore_brings_back() {
        let (shown, _) = sample();
        let mut editor = Editor::new(shown.clone());
        let xy = PrincipalGeometry::Plane(PrincipalPlane::Xy);
        let hide = |geometry| {
            Transaction::single(
                "Hide",
                Edit::SetPrincipalHidden {
                    geometry,
                    hidden: true,
                },
            )
        };

        editor.apply(hide(xy)).unwrap();
        editor.apply(hide(PrincipalGeometry::Origin)).unwrap();
        let hidden = editor.document().clone();
        editor.undo().unwrap();
        let hidden_once = editor.document().clone();
        editor.undo().unwrap();
        let restore = shown.transaction_to(&hidden, "Restore");
        let mut restored = shown.clone();
        restored.apply(restore).unwrap();

        assert!(!hidden.same_content(&shown));
        assert_eq!(
            hidden.hidden_principal().collect::<Vec<_>>(),
            vec![PrincipalGeometry::Origin, xy]
        );
        assert!(hidden_once.is_principal_hidden(xy));
        assert!(!hidden_once.is_principal_hidden(PrincipalGeometry::Origin));
        assert!(editor.document().same_content(&shown));
        assert!(restored.same_content(&hidden));
    }

    #[test]
    fn a_move_past_the_last_feature_is_refused_without_changing_anything() {
        let (mut document, ids) = sample();
        let before = document.clone();
        let count = document.features().count();

        let past_the_end = Transaction::new(
            "Move",
            vec![
                Edit::MoveFeature {
                    id: ids.side,
                    index: 0,
                },
                Edit::MoveFeature {
                    id: ids.base,
                    index: count,
                },
            ],
        );

        assert_eq!(
            document.apply(past_the_end),
            Err(EditError::OutOfRange(count))
        );
        assert_eq!(document, before);
        assert!(
            document
                .apply(Transaction::single(
                    "Move",
                    Edit::MoveFeature {
                        id: ids.base,
                        index: count - 1,
                    },
                ))
                .is_ok()
        );
        assert_eq!(document.features().last().unwrap().id(), ids.base);
    }

    #[test]
    fn id_counters_stop_below_the_storable_range_instead_of_wrapping() {
        let mut document = Document::default();
        document.reserve_ids_below(u64::MAX, u64::MAX);
        assert_eq!(document.next_parameter_id(), FIRST_UNSTORABLE_ID);
        assert_eq!(document.next_feature_id(), FIRST_UNSTORABLE_ID);

        let mut transaction = document.transaction("Add");
        let first = transaction.add_parameter("a", Expression::Number(1.0));
        let second = transaction.add_parameter("b", Expression::Number(2.0));
        assert_eq!(first, ParameterId::from_raw(FIRST_UNSTORABLE_ID));
        assert_eq!(second, ParameterId::from_raw(FIRST_UNSTORABLE_ID + 1));
        assert_eq!(
            document.apply(transaction.finish()).unwrap_err(),
            EditError::ReservedId(FIRST_UNSTORABLE_ID)
        );

        let mut transaction = document.transaction("Add");
        transaction.add_feature("Sketch", FeatureKind::from(Sketch::new(Plane::XY)));
        assert_eq!(
            document.apply(transaction.finish()).unwrap_err(),
            EditError::ReservedId(FIRST_UNSTORABLE_ID)
        );
        assert_eq!(document.parameters().len(), 0);
        assert_eq!(document.next_parameter_id(), FIRST_UNSTORABLE_ID);

        let mut fresh = Document::default();
        let edit = Edit::InsertParameter {
            index: 0,
            parameter: Parameter::new(
                ParameterId::from_raw(u64::MAX),
                "top".to_owned(),
                Expression::Number(0.0),
            ),
        };
        assert_eq!(
            fresh
                .apply(Transaction::single("Insert", edit))
                .unwrap_err(),
            EditError::ReservedId(u64::MAX)
        );
        assert_eq!(fresh.next_parameter_id(), 0);
    }

    #[test]
    fn a_failing_edit_rolls_back_the_whole_transaction() {
        let (mut document, ids) = sample();
        let before = document.clone();
        let transaction = Transaction::new(
            "Two edits",
            vec![
                Edit::RenameParameter {
                    id: ids.width,
                    name: "span".to_owned(),
                },
                Edit::RenameParameter {
                    id: ids.height,
                    name: "span".to_owned(),
                },
            ],
        );

        let error = document.apply(transaction).unwrap_err();

        assert_eq!(error, EditError::DuplicateName("span".to_owned()));
        assert_eq!(document, before);
    }

    #[test]
    fn undo_and_redo_restore_the_exact_document() {
        let (document, ids) = sample();
        let mut editor = Editor::new(document.clone());
        let start = editor.document().clone();

        editor
            .apply(set_expression(editor.document(), ids.gap, "width / 4"))
            .unwrap();
        editor
            .apply(Transaction::single(
                "Delete Side sketch",
                Edit::RemoveFeature { id: ids.side },
            ))
            .unwrap();
        editor
            .apply(Transaction::single(
                "Hide Base sketch",
                Edit::SetFeatureHidden {
                    id: ids.base,
                    hidden: true,
                },
            ))
            .unwrap();
        let end = editor.document().clone();
        assert_eq!(editor.revision(), 3);
        assert_eq!(editor.undo_label(), Some("Hide Base sketch"));

        while editor.undo().unwrap().is_some() {}
        assert_eq!(*editor.document(), start);
        assert_eq!(editor.redo_label(), Some("Edit parameter"));

        while editor.redo().unwrap().is_some() {}
        assert_eq!(*editor.document(), end);
        assert_eq!(editor.revision(), 9);
    }

    #[test]
    fn a_new_edit_clears_redo_and_ids_are_never_reused() {
        let (document, _) = sample();
        let mut editor = Editor::new(document);
        let add = |editor: &Editor, name: &str| {
            let mut transaction = editor.document().transaction("Add parameter");
            let id = transaction.add_parameter(name, Expression::Number(1.0));
            (id, transaction.finish())
        };

        let (first, transaction) = add(&editor, "first");
        editor.apply(transaction).unwrap();
        editor.undo().unwrap();
        let (second, transaction) = add(&editor, "second");
        editor.apply(transaction).unwrap();

        assert_ne!(first, second);
        assert_eq!(editor.redo_label(), None);
        assert!(editor.document().parameter(first).is_none());
    }

    #[test]
    fn removing_a_parameter_in_use_names_its_users() {
        let (mut document, ids) = sample();
        document
            .apply(set_expression(&document, ids.gap, "width / 8"))
            .unwrap();

        let error = document
            .apply(Transaction::single(
                "Delete width",
                Edit::RemoveParameter { id: ids.width },
            ))
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "width is used by gap and Base sketch. Remove those uses first."
        );
    }

    #[test]
    fn an_expression_that_would_form_a_cycle_is_refused() {
        let (mut document, ids) = sample();
        document
            .apply(set_expression(&document, ids.height, "width / 2"))
            .unwrap();
        document
            .apply(set_expression(&document, ids.gap, "height / 2"))
            .unwrap();

        let error = document
            .apply(set_expression(&document, ids.width, "gap * 8"))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "This would make width depend on itself (width → gap → height → width)"
        );
        let error = document
            .apply(set_expression(&document, ids.width, "width + 1"))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "This would make width depend on itself (width → width)"
        );
    }

    #[test]
    fn names_are_checked_and_references_follow_renames() {
        let (mut document, ids) = sample();
        let rename = |id, name: &str| {
            Transaction::single(
                "Rename",
                Edit::RenameParameter {
                    id,
                    name: name.to_owned(),
                },
            )
        };
        document
            .apply(set_expression(&document, ids.gap, "width / 8"))
            .unwrap();

        assert!(matches!(
            document.apply(rename(ids.width, "mm")),
            Err(EditError::InvalidName(_))
        ));
        assert!(matches!(
            document.apply(rename(ids.width, "height")),
            Err(EditError::DuplicateName(_))
        ));
        document.apply(rename(ids.width, "span")).unwrap();

        let gap = document.parameter(ids.gap).unwrap();
        assert_eq!(document.expression_text(&gap.expression), "span / 8");
        assert!(document.parse("width").is_err());
    }

    #[test]
    fn feature_names_cannot_be_blank_and_are_trimmed() {
        let (mut document, ids) = sample();
        let rename = |name: &str| {
            Transaction::single(
                "Rename",
                Edit::RenameFeature {
                    id: ids.base,
                    name: name.to_owned(),
                },
            )
        };
        assert_eq!(
            document.apply(rename("   ")),
            Err(EditError::EmptyFeatureName)
        );
        document.apply(rename("  Outline ")).unwrap();
        assert_eq!(document.feature(ids.base).unwrap().name, "Outline");
        document.apply(rename("Outline")).unwrap();
    }

    #[test]
    fn feature_names_are_unique() {
        let (mut document, ids) = sample();
        let taken = document
            .features()
            .find(|feature| feature.id() != ids.base)
            .map(|feature| feature.name.clone())
            .unwrap();
        assert_eq!(
            document.apply(Transaction::single(
                "Rename",
                Edit::RenameFeature {
                    id: ids.base,
                    name: taken.clone(),
                },
            )),
            Err(EditError::DuplicateFeatureName(taken.clone()))
        );
        let mut transaction = document.transaction("Add");
        transaction.add_feature(taken.clone(), FeatureKind::from(Sketch::new(Plane::XY)));
        assert_eq!(
            document.apply(transaction.finish()),
            Err(EditError::DuplicateFeatureName(taken.clone()))
        );

        let mut transaction = document.transaction("Add");
        transaction.add_feature(
            format!(" {taken}\t"),
            FeatureKind::from(Sketch::new(Plane::XY)),
        );
        assert_eq!(
            document.apply(transaction.finish()),
            Err(EditError::DuplicateFeatureName(taken))
        );
        let mut transaction = document.transaction("Add");
        let padded = transaction.add_feature("  Top  ", FeatureKind::from(Sketch::new(Plane::XY)));
        document.apply(transaction.finish()).unwrap();
        assert_eq!(document.feature(padded).unwrap().name, "Top");
    }

    #[test]
    fn a_sketch_point_added_and_undone_before_recompute_reuses_the_result() {
        let (document, ids) = sample();
        let mut editor = Editor::new(document);
        let mut engine = Recompute::default();

        recompute(&mut engine, editor.document());
        let point = caditor_sketch::EntityId::from_raw(
            editor
                .document()
                .feature(ids.base)
                .and_then(|feature| feature.kind.sketch())
                .unwrap()
                .next_id(),
        );
        editor
            .apply(Transaction::single(
                "Add point",
                Edit::AddSketchEntity {
                    feature: ids.base,
                    id: point,
                    entity: Entity::Point(Point2::ZERO),
                    construction: false,
                },
            ))
            .unwrap();
        editor.undo().unwrap();
        let undone = recompute(&mut engine, editor.document());

        assert!(undone.recomputed().is_empty());
    }

    #[test]
    fn check_reports_the_error_without_changing_the_document() {
        let (document, ids) = sample();
        let transaction =
            Transaction::single("Delete width", Edit::RemoveParameter { id: ids.width });
        assert!(document.check(&transaction).is_err());
        assert!(document.parameter(ids.width).is_some());
    }

    #[test]
    fn a_parameter_is_removable_once_the_same_transaction_stops_using_it() {
        let (mut document, ids) = sample();
        let before = document.clone();
        let mut transaction = document.transaction("Chain");
        let wide = transaction.add_parameter("wide", transaction.parse("width + 1 mm").unwrap());
        let wider = transaction.add_parameter("wider", Expression::Parameter(wide));
        document.apply(transaction.finish()).unwrap();
        let drop_wide = |document: &Document| {
            Transaction::new(
                "Drop",
                vec![
                    Edit::SetParameterExpression {
                        id: wider,
                        expression: document.parse("1 mm").unwrap(),
                    },
                    Edit::RemoveParameter { id: wide },
                ],
            )
        };
        let still_used = Transaction::single("Drop", Edit::RemoveParameter { id: wide });
        let snapshot = document.clone();

        assert!(matches!(
            document.apply(still_used),
            Err(EditError::ParameterInUse { .. })
        ));
        assert_eq!(document, snapshot);

        let inverse = document.apply(drop_wide(&document)).unwrap();

        assert!(document.parameter(wide).is_none());
        assert!(document.parameter_named("wide").is_none());
        assert_eq!(
            document.parameter_named("wider").map(Parameter::id),
            Some(wider)
        );
        assert!(document.parameter(ids.width).is_some());

        document.apply(inverse).unwrap();

        assert!(document.same_content(&snapshot));
        assert!(!document.same_content(&before));
        assert_eq!(
            document.parameter_named("wide").map(Parameter::id),
            Some(wide)
        );
    }

    fn dimension_as(document: &Document, ids: &Ids, text: &str) -> Transaction {
        Transaction::single(
            "Set dimension",
            Edit::SetDimension {
                feature: ids.base,
                constraint: ids.base_distance,
                value: document.parse(text).unwrap(),
            },
        )
    }

    fn found_regions(evaluation: &Evaluation, feature: FeatureId) -> bool {
        evaluation
            .feature(feature)
            .and_then(|status| status.result.as_deref())
            .and_then(FeatureResult::sketch)
            .is_some_and(|sketch| sketch.regions().is_some())
    }

    #[test]
    fn a_sketch_that_solves_to_the_same_geometry_keeps_its_found_regions() {
        let (mut document, ids) = sample();
        let mut engine = Recompute::default();
        let first = recompute(&mut engine, &document);
        first
            .feature(ids.base)
            .and_then(|status| status.result.as_deref())
            .and_then(FeatureResult::sketch)
            .unwrap()
            .find_regions();
        assert!(found_regions(&first, ids.base));

        document
            .apply(dimension_as(&document, &ids, "40 mm"))
            .unwrap();
        let same = recompute(&mut engine, &document);
        assert!(same.recomputed().contains(&ids.base));
        assert!(found_regions(&same, ids.base));

        document
            .apply(dimension_as(&document, &ids, "30 mm"))
            .unwrap();
        let moved = recompute(&mut engine, &document);
        assert!(moved.recomputed().contains(&ids.base));
        assert!(found_regions(&moved, ids.base));
    }

    #[test]
    fn recompute_only_revisits_features_whose_inputs_changed() {
        let (mut document, ids) = sample();
        let mut engine = Recompute::default();

        let first = recompute(&mut engine, &document);
        assert_eq!(first.recomputed(), &[ids.base, ids.side]);

        document
            .apply(set_expression(&document, ids.gap, "7 mm"))
            .unwrap();
        assert!(recompute(&mut engine, &document).recomputed().is_empty());

        document
            .apply(set_expression(&document, ids.width, "20 mm + 2 cm"))
            .unwrap();
        assert!(recompute(&mut engine, &document).recomputed().is_empty());

        document
            .apply(set_expression(&document, ids.height, "25 mm"))
            .unwrap();
        let evaluation = recompute(&mut engine, &document);
        assert_eq!(evaluation.recomputed(), &[ids.side]);
        let Some(FeatureResult::Sketch(result)) = evaluation
            .feature(ids.side)
            .and_then(|status| status.result.as_deref())
        else {
            panic!("the side sketch should have a result");
        };
        assert_eq!(result.solution.dimension(ids.side_distance), Some(25.0));
    }

    #[test]
    fn a_failing_feature_keeps_its_last_good_result_and_others_are_unaffected() {
        let (mut document, ids) = sample();
        let mut engine = Recompute::default();
        let good = recompute(&mut engine, &document);
        let good_result = good.feature(ids.base).unwrap().result.clone();

        document
            .apply(set_expression(&document, ids.width, "10 deg"))
            .unwrap();
        let evaluation = recompute(&mut engine, &document);

        let error = failure(&evaluation, ids.base);
        assert_eq!(
            error.reason,
            "Distance between Point 0 and Point 1 cannot be evaluated: it gives an angle, but a \
             length is needed."
        );
        assert_eq!(
            error.fix,
            Some(FixTarget::Dimension {
                feature: ids.base,
                constraint: ids.base_distance
            })
        );
        assert_eq!(evaluation.feature(ids.base).unwrap().result, good_result);
        assert_eq!(
            evaluation.feature(ids.side).unwrap().state,
            FeatureState::UpToDate
        );
        assert_eq!(evaluation.failed_count(), 1);

        document
            .apply(set_expression(&document, ids.width, "30 mm"))
            .unwrap();
        let evaluation = recompute(&mut engine, &document);
        assert_eq!(evaluation.failed_count(), 0);
    }

    #[test]
    fn a_broken_parameter_points_the_fix_at_the_parameter() {
        let (mut document, ids) = sample();
        document
            .apply(set_expression(
                &document,
                ids.width,
                "1 mm / (gap - 5 mm) * 1 mm",
            ))
            .unwrap();
        let evaluation = recompute(&mut Recompute::default(), &document);

        assert_eq!(
            evaluation.parameters.get(ids.width),
            Some(&Err(ParameterError::Evaluation(EvalError::DivisionByZero)))
        );
        let error = failure(&evaluation, ids.base);
        assert_eq!(
            error.reason,
            "Distance between Point 0 and Point 1 cannot be evaluated: it uses width, which has \
             an error."
        );
        assert_eq!(
            error.remedy,
            "Fix width under Parameters, or edit this dimension."
        );
        assert_eq!(error.fix, Some(FixTarget::Parameter(ids.width)));
    }

    #[test]
    fn parameter_cycles_from_damaged_data_are_reported_not_followed() {
        let (mut document, ids) = sample();
        let cyclic = |id: ParameterId, uses: ParameterId| {
            (
                id,
                Expression::binary(
                    caditor_expression::BinaryOperator::Add,
                    Expression::Parameter(uses),
                    Expression::Number(1.0),
                ),
            )
        };
        for (id, expression) in [cyclic(ids.width, ids.height), cyclic(ids.height, ids.width)] {
            document.parameters.replace_expression(id, expression);
        }
        document
            .apply(set_expression(&document, ids.gap, "width * 2"))
            .unwrap();

        let values = ParameterValues::evaluate(&document);
        assert_eq!(
            values.get(ids.width),
            Some(&Err(ParameterError::Cycle {
                path: "width → height → width".to_owned()
            }))
        );
        assert_eq!(
            values.value(ids.gap),
            Err(EvalError::ParameterFailed {
                id: ids.gap,
                name: "gap".to_owned()
            })
        );
        assert!(matches!(
            values.get(ids.gap),
            Some(Err(ParameterError::Evaluation(
                EvalError::ParameterFailed { .. }
            )))
        ));
    }

    #[test]
    fn parameter_values_evaluate_in_dependency_order() {
        let (mut document, ids) = sample();
        document
            .apply(set_expression(&document, ids.width, "gap * 8"))
            .unwrap();
        document
            .apply(set_expression(&document, ids.gap, "height / 4"))
            .unwrap();

        let values = ParameterValues::evaluate(&document);
        assert_eq!(values.value(ids.width), Ok(Quantity::length(40.0)));
        assert_eq!(
            values.evaluate_expression(&document.parse("width + gap").unwrap()),
            Ok(Quantity::length(45.0))
        );
    }

    #[test]
    fn a_solved_sketch_follows_its_dimension() {
        let (mut document, ids) = sample();
        document
            .apply(set_expression(&document, ids.width, "55 mm"))
            .unwrap();
        let evaluation = recompute(&mut Recompute::default(), &document);
        let Some(FeatureResult::Sketch(result)) = evaluation
            .feature(ids.base)
            .and_then(|status| status.result.as_deref())
        else {
            panic!("the base sketch should have a result");
        };
        let line = result
            .geometry
            .entities()
            .find_map(|(id, entity)| matches!(entity, Entity::Line { .. }).then_some(id))
            .unwrap();
        let (start, end) = result.geometry.line_endpoints(line).unwrap();
        assert!((start.distance(end) - 55.0).abs() < 1e-9);
        assert_eq!(result.solution.degrees_of_freedom(), 2);
    }

    #[test]
    fn every_conflicting_part_of_a_sketch_is_reported_together() {
        let mut sketch = Sketch::new(Plane::XY);
        let mut conflicts = Vec::new();
        for row in 0..2 {
            let y = f64::from(row) * 20.0;
            let line = sketch.add_line(Point2::new(0.0, y), Point2::new(10.0, y));
            let horizontal = sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
            let vertical = sketch.add_constraint(Constraint::Vertical(line)).unwrap();
            conflicts.push((horizontal, vertical));
        }
        let mut document = Document::default();
        let mut transaction = document.transaction("Add sketch");
        let feature = transaction.add_feature("Base sketch", FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();

        let evaluation = recompute(&mut Recompute::default(), &document);

        let error = failure(&evaluation, feature);
        assert!(
            error
                .reason
                .starts_with("The sketch has 2 separate problems. (1) ")
        );
        assert!(error.reason.contains(" (2) "));
        assert!(error.remedy.starts_with("Fix each of them. (1) "));
        assert_eq!(
            error.constraints,
            [
                conflicts[1].0,
                conflicts[1].1,
                conflicts[0].0,
                conflicts[0].1
            ]
        );
        assert_eq!(
            error.fix,
            Some(FixTarget::Constraint {
                feature,
                constraint: conflicts[1].1
            })
        );
    }

    #[test]
    fn conflicting_constraints_are_named_and_the_fix_points_at_the_newest() {
        let (mut sketch, _) = line_with_distance(Plane::XY, 40.0, Expression::Number(40.0));
        let line = sketch
            .entities()
            .find_map(|(id, entity)| matches!(entity, Entity::Line { .. }).then_some(id))
            .unwrap();
        let vertical = sketch.add_constraint(Constraint::Vertical(line)).unwrap();
        let horizontal = sketch
            .constraints()
            .find_map(|(id, constraint)| {
                matches!(constraint, Constraint::Horizontal(_)).then_some(id)
            })
            .unwrap();
        let mut document = Document::default();
        let mut transaction = document.transaction("Add sketches");
        let replaced = transaction.add_feature("Base sketch", FeatureKind::from(sketch));
        let (fine, _) = line_with_distance(Plane::XZ, 20.0, Expression::Number(20.0));
        let side = transaction.add_feature("Side sketch", FeatureKind::from(fine));
        document.apply(transaction.finish()).unwrap();

        let evaluation = recompute(&mut Recompute::default(), &document);

        let error = failure(&evaluation, replaced);
        assert_eq!(
            error.reason,
            "Vertical Line 2 conflicts with Horizontal Line 2."
        );
        assert_eq!(
            error.remedy,
            "Delete or change one of these constraints, or undo the last change."
        );
        assert_eq!(
            error.fix,
            Some(FixTarget::Constraint {
                feature: replaced,
                constraint: vertical
            })
        );
        assert_eq!(error.constraints, [horizontal, vertical]);
        assert_eq!(
            evaluation.feature(side).unwrap().state,
            FeatureState::UpToDate
        );
    }

    #[test]
    fn added_constraint_kinds_are_named_in_conflicts_and_dimension_errors() {
        let mut sketch = Sketch::new(Plane::XY);
        let point = sketch.add_point(Point2::new(4.0, 1.0));
        let circle = sketch.add_circle(Point2::new(20.0, 0.0), 3.0);
        let fixed = sketch
            .add_constraint(Constraint::Fix {
                point,
                at: Point2::new(4.0, 1.0),
            })
            .unwrap();
        let level = sketch
            .add_constraint(Constraint::HorizontalPoints(point, EntityId::ORIGIN))
            .unwrap();
        let diameter = sketch
            .add_constraint(Constraint::Diameter {
                entity: circle,
                value: Expression::Number(6.0),
            })
            .unwrap();
        let mut document = Document::default();
        let mut transaction = document.transaction("Add sketch");
        let feature = transaction.add_feature("Holes", FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();

        let evaluation = recompute(&mut Recompute::default(), &document);

        let error = failure(&evaluation, feature);
        assert_eq!(
            error.reason,
            "Horizontal Point 0 and Origin conflicts with Fix Point 0."
        );
        assert_eq!(error.constraints, [fixed, level]);

        document
            .apply(Transaction::single(
                "Remove",
                Edit::RemoveSketchConstraint { feature, id: level },
            ))
            .unwrap();
        document
            .apply(Transaction::single(
                "Zero",
                Edit::SetDimension {
                    feature,
                    constraint: diameter,
                    value: Expression::Number(0.0),
                },
            ))
            .unwrap();
        let evaluation = recompute(&mut Recompute::default(), &document);
        let error = failure(&evaluation, feature);
        assert_eq!(
            error.reason,
            "Diameter of Circle 2 cannot be evaluated: a diameter must be greater than zero."
        );
        assert_eq!(
            error.remedy,
            "Edit the dimension so it gives more than zero."
        );
    }

    #[test]
    fn an_unsolved_sketch_names_its_geometry_and_points_at_the_newest_constraint() {
        let mut sketch = Sketch::new(Plane::XY);
        let lines: Vec<_> = (0..5)
            .map(|index| {
                let x = f64::from(index);
                sketch.add_line(Point2::new(x, 0.0), Point2::new(x + 1.0, 0.0))
            })
            .collect();
        let newest = sketch
            .add_constraint(Constraint::Horizontal(lines[4]))
            .unwrap();
        let feature = FeatureId::from_raw(7);

        let many = recompute::unsolvable_error(feature, &sketch, &lines, Some(newest));
        let one = recompute::unsolvable_error(feature, &sketch, &lines[..1], None);
        let unknown = recompute::unsolvable_error(feature, &sketch, &[], None);

        assert_eq!(
            many.reason,
            "Line 2, Line 5, Line 8 and 2 more could not be solved from their current shape."
        );
        assert_eq!(
            many.remedy,
            "Delete or change Horizontal Line 14, the newest constraint on them, or undo the last \
             change."
        );
        assert_eq!(
            many.fix,
            Some(FixTarget::Constraint {
                feature,
                constraint: newest
            })
        );
        assert_eq!(
            one.reason,
            "Line 2 could not be solved from its current shape."
        );
        assert_eq!(one.remedy, "Delete and redraw it, or undo the last change.");
        assert_eq!(one.fix, Some(FixTarget::Feature(feature)));
        assert_eq!(
            unknown.reason,
            "The sketch could not be solved from its current shape."
        );
        assert_eq!(unknown.remedy, "Undo the last change.");
    }

    #[test]
    fn a_cancelled_solve_leaves_the_feature_outdated() {
        let (mut document, ids) = sample();
        document
            .apply(set_expression(&document, ids.width, "50 mm"))
            .unwrap();
        let checks = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&checks);
        let cancel = CancelToken::new(move || counter.fetch_add(1, Ordering::SeqCst) >= 1);

        let evaluation = Recompute::default().run(&document, &ModelEvaluator, &cancel, &|_, _| {});

        assert_eq!(
            evaluation.feature(ids.base).unwrap().state,
            FeatureState::Outdated
        );
        assert!(checks.load(Ordering::SeqCst) >= 2);
        assert!(!evaluation.is_complete());
    }
}

#[cfg(test)]
mod scale_tests {
    use std::time::{Duration, Instant};

    use caditor_expression::{BinaryOperator, Expression, Unit};

    use crate::{Document, ParameterValues};

    const CHAIN: usize = 1200;
    const GROWTH: usize = 4;
    const RUNS: usize = 3;
    const LINEAR_SLACK: f64 = 2.0;

    fn chain(length: usize, step: f64) -> Document {
        let mut document = Document::default();
        let mut transaction = document.transaction("Chain");
        let mut previous =
            transaction.add_parameter("p0", Expression::Measure(1.0, Unit::Millimetre));
        for index in 1..length {
            previous = transaction.add_parameter(
                format!("p{index}"),
                Expression::binary(
                    BinaryOperator::Add,
                    Expression::Parameter(previous),
                    Expression::Measure(step, Unit::Millimetre),
                ),
            );
        }
        document.apply(transaction.finish()).unwrap();
        document
    }

    fn evaluate_and_restore(length: usize) -> Duration {
        let started = Instant::now();
        let mut document = chain(length, 1.0);

        let values = ParameterValues::evaluate(&document);
        let last = document.parameters().last().unwrap().id();
        assert_eq!(values.value(last).unwrap().value, length as f64);

        let other = chain(length, 2.0);
        let restore = document.transaction_to(&other, "Restore");
        document.apply(restore).unwrap();
        assert!(document.same_content(&other));

        started.elapsed()
    }

    fn fastest_of_runs(length: usize) -> Duration {
        (0..RUNS)
            .map(|_| evaluate_and_restore(length))
            .min()
            .unwrap()
    }

    #[test]
    fn long_parameter_chains_evaluate_and_restore_in_linear_time() {
        let short = fastest_of_runs(CHAIN / GROWTH);
        let long = fastest_of_runs(CHAIN);

        let ratio = long.as_secs_f64() / short.as_secs_f64().max(f64::MIN_POSITIVE);
        assert!(
            ratio < GROWTH as f64 * LINEAR_SLACK,
            "{GROWTH} times the chain took {ratio:.1} times as long ({short:?} and {long:?})"
        );
    }
}
