use std::collections::BTreeSet;

use caditor_expression::{EvalError, ParameterId, Quantity};

use crate::{
    constraint::Constraint,
    id::ConstraintId,
    sketch::{DimensionValues, Sketch, SketchError},
    solve::{
        numeric::{Component, Parts, STIFF, Solver},
        system::System,
    },
};

impl Sketch {
    pub fn determines(
        &self,
        dimensions: &DimensionValues,
        parameters: &dyn Fn(ParameterId) -> Result<Quantity, EvalError>,
        dimension: &Constraint,
    ) -> Result<bool, SketchError> {
        let (Some(expression), Some(kind)) = (dimension.dimension(), dimension.dimension_kind())
        else {
            return Ok(false);
        };
        let Ok(value) = expression.evaluate_as(kind, &parameters) else {
            return Ok(false);
        };

        let mut probed = self.clone();
        let added = probed.add_constraint(dimension.clone())?;
        let dimensions = dimensions.clone().with_dimension(added, value);
        let system = System::build(&probed, &dimensions)?;
        let parts = Parts::of(&system);

        let never = || false;
        let held = BTreeSet::new();
        let solver = Solver {
            system: &system,
            cancelled: &never,
            stiff: &held,
            stiffness: STIFF,
            retrying: true,
        };
        let measuring: Vec<&Component> = parts
            .drawn()
            .iter()
            .filter(|component| owns_an_equation(&system, component, added))
            .collect();
        if measuring.is_empty() {
            return Ok(false);
        }
        for component in measuring {
            if solver.adds_rank_over_the_rest(component, &system.values, added)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn owns_an_equation(system: &System, component: &Component, owner: ConstraintId) -> bool {
    component.equations.iter().any(|index| {
        system
            .equations
            .get(*index)
            .is_some_and(|equation| equation.owner == Some(owner))
    })
}
