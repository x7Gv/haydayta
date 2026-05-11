use std::collections::{HashMap, HashSet};
use good_lp::{constraint, default_solver, variable, variables, Expression, Solution, SolverModel};
use crate::domain::{GoodId, Process, ProcessId, ProductionPlan, Source};

pub fn solve_production_plan<F>(
    processes: &[Process],
    sources: &[Source],
    initial_inventory: &HashMap<GoodId, u64>,
    benefit_fn: F,
    requirements: HashMap<GoodId, u64>,
) -> Result<ProductionPlan, String>
where
    F: Fn(&Process) -> f64
{
    let mut context = variables!();

    let vars: HashMap<ProcessId, _> = processes
        .iter()
        .map(|p| {
            let var = context.add(variable().integer().min(0).name(&p.id.0));
            (p.id.clone(), var)
        })
        .collect();

    let objective: Expression = processes
        .iter()
        .map(|p| vars[&p.id] * benefit_fn(p))
        .sum();

    let mut model = context.maximise(objective.clone()).using(default_solver);

    for source in sources {
        let time_spent: Expression = processes.iter()
            .filter(|p| p.source_id == source.id)
            .map(|p| vars[&p.id] * p.duration.as_secs_f64())
            .sum();

        model.add_constraint(constraint!(time_spent <= source.capacity.as_secs_f64()));
    }

    let all_goods: HashSet<GoodId> = processes.iter()
        .flat_map(|p| p.needs.iter().map(|(id, _)| id.clone())
            .chain(std::iter::once(p.product.0.clone()))
        ).collect();

    for good_id in all_goods {
        let net_change: Expression = processes.iter()
            .map(|p| {
                let mut coeff = 0.0;
                if p.product.0 == good_id { coeff += p.product.1 as f64; }
                if let Some((_, amount)) = p.needs.iter().find(|(id, _)| *id == good_id) {
                    coeff -= *amount as f64;
                }
                vars[&p.id] * coeff
            })
            .sum();

        if let Some(amount) = requirements.get(&good_id) {
            model.add_constraint(constraint!(net_change.clone() <= (*amount as f64)));
        }

        let initial = *initial_inventory.get(&good_id).unwrap_or(&0) as f64;
        model.add_constraint(constraint!(net_change + initial >= 0.0));
    }

    let solution = model.solve().map_err(|e| e.to_string())?;

    let counts = processes
        .iter()
        .map(|p| (p.id.clone(), solution.value(vars[&p.id]) as u64))
        .collect();

    Ok(ProductionPlan {
        counts,
        total_value: solution.eval(objective),
    })
}