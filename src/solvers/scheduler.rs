use std::collections::{HashMap, VecDeque};
use good_lp::{constraint, default_solver, variable, variables, Solution, SolverModel, WithTimeLimit};
use petgraph::algo::is_cyclic_directed;
use petgraph::visit::Topo;
use crate::domain::{DependencyGraph, Duration, GoodId, ProcessId, ProductionPlan, Registry, SourceId};

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct JobId(pub String);

#[derive(Clone, Debug)]
pub struct Job {
    pub job_id: JobId,
    pub good_id: GoodId,
    pub machine_id: SourceId,
    pub duration: Duration,
    pub consumes: Vec<JobId>,
}

pub fn generate_jobs(
    registry: &Registry,
    graph: &DependencyGraph,
    production_plan: ProductionPlan,
) -> Vec<Job> {
    let mut final_jobs = Vec::new();
    let mut available_supply: HashMap<GoodId, VecDeque<JobId>> = HashMap::new();

    for (item_id, &quantity) in &production_plan.counts {
        let mut queue = VecDeque::new();
        for i in 1..=quantity {
            queue.push_back(JobId(format!("{}_{}", item_id.0, i)));
        }
        available_supply.insert(GoodId(item_id.0.clone()), queue);
    }

    println!("{:?}", is_cyclic_directed(&graph.inner));

    let mut sorted_items = Topo::new(&graph.inner);

    while let Some(good) = sorted_items.next(&graph.inner) {

        let good_id = graph.inner.node_weight(good).unwrap();

        let quantity = match production_plan.counts.get(&ProcessId(good_id.0.clone())) {
            Some(&q) => q,
            None => continue,
        };

        let recipe = graph.dependencies(&good_id);
        let good = registry.goods.get(&good_id).expect("Good not found");
        let process_id = ProcessId(good_id.0.clone());
        let process = registry.processes.get(&process_id).expect("Process not found");

        for i in 1..=quantity {
            let job_id = JobId(format!("{}_{}", good_id.0, i));
            let mut dependencies = Vec::new();

            for (ingredient_id, need_quantity) in &recipe {
                for _ in 0..*need_quantity {
                    if let Some(supply_queue) = available_supply.get_mut(ingredient_id) {
                        if let Some(dep_job_id) = supply_queue.pop_front() {
                            dependencies.push(dep_job_id);
                        }
                    }
                }
            }

            final_jobs.push(Job {
                job_id,
                machine_id: good.source.clone(),
                good_id: good_id.clone(),
                duration: process.duration,
                consumes: dependencies,
            });
        }
    }

    final_jobs
}

pub struct Scheduler {
    jobs: Vec<Job>,
}

impl Scheduler {
    pub fn new(jobs: Vec<Job>) -> Self {
        Self { jobs }
    }

    pub fn solve(&self) -> Option<(f64, Vec<((Job, f64))>)> {
        let mut vars = variables!();

        let makespan = vars.add(variable().min(0.0));

        let mut start_times = HashMap::new();
        let mut job_map = HashMap::<JobId, Job>::new();

        for job in &self.jobs {
            start_times.insert(job.job_id.clone(), vars.add(variable().min(0.0)));
            job_map.insert(job.job_id.clone(), job.clone());
        }

        let mut machine_jobs: HashMap<SourceId, Vec<Job>> = HashMap::new();
        for job in &self.jobs {
            machine_jobs.entry(job.machine_id.clone()).or_default().push(job.clone());
        }

        let mut ordering_vars = HashMap::new();

        for (_, m_jobs) in &machine_jobs {
            for i in 0..m_jobs.len() {
                for j in (i+1)..m_jobs.len() {
                    let job_a = &m_jobs[i];
                    let job_b = &m_jobs[j];

                    let bin_var = vars.add(variable().binary());
                    ordering_vars.insert((job_a.job_id.clone(), job_b.job_id.clone()), bin_var);
                }
            }
        }

        let mut model = vars.minimise(makespan).using(default_solver);

        for job in &self.jobs {
            let start = start_times[&job.job_id];
            model = model.with(constraint!(start + job.duration.as_secs_f64() <= makespan));
        }

        for job in &self.jobs {
            let start = start_times[&job.job_id];
            for dep_id in &job.consumes {
                let dep_start = start_times[dep_id];
                let dep_duration = job_map[dep_id].duration;

                model = model.with(constraint!(start >= dep_start + dep_duration.as_secs_f64()));
            }
        }

        let big_m: f64 = self.jobs.iter().map(|j| j.duration.as_secs_f64()).sum();

        for ((id_a, id_b), bin_var) in ordering_vars.into_iter() {
            let start_a = start_times[&id_a];
            let start_b = start_times[&id_b];
            let dur_a = job_map[&id_a].duration;
            let dur_b = job_map[&id_b].duration;

            if (job_map.get(&id_a)?.good_id == job_map.get(&id_b)?.good_id) {
                model = model.with(constraint!(start_b >= start_a + dur_a.as_secs_f64()));
                continue;
            }

            model = model.with(constraint!(
                start_b + big_m - (big_m * bin_var) >= start_a + dur_a.as_secs_f64()
            ));

            model = model.with(constraint!(
                start_a + (big_m * bin_var) >= start_b + dur_b.as_secs_f64()
            ));
        }

        model = model.with_time_limit(300);

        let solution = model.solve().ok()?;

        let mut schedule = HashMap::new();
        for (id, var) in start_times {
            schedule.insert(id, solution.value(var));
        }

        let sol = schedule.into_iter()
            .map(|(id, t)| (job_map.get(&id).unwrap().clone(), t))
            .collect::<Vec<_>>();

        Some((solution.value(makespan), sol))
    }
}
