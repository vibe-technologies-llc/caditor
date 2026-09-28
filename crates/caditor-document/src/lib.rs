mod attachment;
mod blend;
mod datum;
mod describe;
mod document;
mod edit;
mod editor;
mod recompute;
mod shell;
mod solid;
mod values;
mod worker;

pub use crate::{
    attachment::{AttachmentError, FaceAttachment, SketchAttachment, SketchFeature, face_plane},
    blend::{Blend, BlendKind},
    datum::{
        AxisReference, Datum, DatumAxis, DatumPlane, DatumResult, PlaneReference, PlaneRotation,
        PrincipalAxis, PrincipalPlane, capitalized, describe_axis, describe_plane, displayed_axis,
    },
    describe::{describe_edge, describe_origin, edge_faces, origin_feature},
    document::{Document, Feature, FeatureId, FeatureKind, Parameter},
    edit::{Edit, EditError, Transaction, TransactionBuilder},
    editor::Editor,
    recompute::{
        CancelToken, Evaluation, Evaluator, Failure, FeatureError, FeatureResult, FeatureState,
        FeatureStatus, FixTarget, Inputs, ModelEvaluator, Recompute, SketchResult,
    },
    shell::Shell,
    solid::{
        BodyOperation, Extrude, ExtrudeExtent, RegionChoice, Revolve, RevolveAxis, RevolveExtent,
        SketchRegion, SolidFeature, SolidResult, sketch_regions,
    },
    values::{ParameterError, ParameterValues},
    worker::{Outcome, Progress, Recomputer, Update, WorkerStopped},
};

#[cfg(test)]
mod attachment_tests;
#[cfg(test)]
mod blend_tests;
#[cfg(test)]
mod datum_tests;
#[cfg(test)]
mod shell_tests;
#[cfg(test)]
mod sketch_tests;
#[cfg(test)]
mod solid_tests;
#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use caditor_expression::{EvalError, Expression, ParameterId, Quantity};
    use caditor_geometry::{Plane, Point2};
    use caditor_sketch::{Constraint, ConstraintId, Entity, Sketch};

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
                "Move Side sketch",
                Edit::MoveFeature {
                    id: ids.base,
                    index: 0,
                },
            ))
            .unwrap();
        let end = editor.document().clone();
        assert_eq!(editor.revision(), 3);
        assert_eq!(editor.undo_label(), Some("Move Side sketch"));

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
            if let Some(parameter) = document.parameters.iter_mut().find(|p| p.id() == id) {
                parameter.expression = expression;
            }
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
    fn conflicting_constraints_are_named_and_the_fix_points_at_the_newest() {
        let (mut sketch, distance) = line_with_distance(Plane::XY, 40.0, Expression::Number(40.0));
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
            "Vertical Line 2 conflicts with Horizontal Line 2 and Distance between Point 0 and \
             Point 1."
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
        assert_eq!(error.constraints, [horizontal, distance, vertical]);
        assert_eq!(
            evaluation.feature(side).unwrap().state,
            FeatureState::UpToDate
        );
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
