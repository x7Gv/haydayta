mod solvers;
mod parsers;

use crate::domain::{DependencyGraph, Registry, Scenario, Source, SourceId};
use crate::parsers::{needs, source_id, time, TimeUnit};
use crate::solvers::scheduler::{generate_jobs, Job, Scheduler};
use petgraph::prelude::*;
use scraper::Selector;
use std::collections::HashMap;
use std::time::Duration;
use tabular::{Row, Table};

pub mod domain {
    use petgraph::graph::NodeIndex;
    use petgraph::prelude::EdgeRef;
    use petgraph::Graph;
    use std::collections::{HashMap, HashSet};
    use std::fmt::Display;
    use std::time;

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    pub struct GoodId(pub String);

    impl Display for GoodId {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    pub struct SourceId(pub String);

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    pub struct ProcessId(pub String);

    pub type Level = u64;
    pub type Experience = u64;
    pub type Duration = time::Duration;

    #[derive(Debug, Clone)]
    pub struct Good {
        pub id: GoodId,
        pub level: Level,
        pub xp: Experience,
        pub source: SourceId,
        pub price: u64,
        pub image_url: Option<url::Url>,
    }

    #[derive(Debug, Clone)]
    pub struct Process {
        pub id: ProcessId,
        pub source_id: SourceId,
        pub duration: time::Duration,
        pub goods: Vec<Good>,
        pub product: (GoodId, u64),
        pub needs: Vec<(GoodId, u64)>
    }

    #[derive(Debug, Clone)]
    pub struct Source {
        pub id: SourceId,
        pub capacity: Duration,
    }

    #[derive(Debug, Clone)]
    pub struct ProductionPlan {
        pub counts: HashMap<ProcessId, u64>,
        pub total_value: f64,
    }

    #[derive(Debug, Clone)]
    pub struct Registry {
        pub goods: HashMap<GoodId, Good>,
        pub processes: HashMap<ProcessId, Process>,
        pub sources: HashMap<SourceId, Source>,
    }

    #[derive(Debug, Clone)]
    pub struct Scenario<'a> {
        pub registry: &'a Registry,
        pub level: u64,
        pub enabled_sources: HashSet<SourceId>,
        pub target_goods: HashSet<GoodId>,
    }

    impl<'a> Scenario<'a> {
        pub fn new(registry: &'a Registry) -> Self {
            Self {
                registry,
                level: u64::MAX,
                enabled_sources: HashSet::new(),
                target_goods: HashSet::new(),
            }
        }

        pub fn limit_level(mut self, level: u64) -> Self {
            self.level = level;
            self
        }

        pub fn with_sources(mut self, ids: &[&str]) -> Self {
            self.enabled_sources.extend(ids.iter().map(|&id| SourceId(id.to_string())));
            self
        }

        pub fn with_all_sources_filtered<F: FnMut(&Source) -> bool>(mut self, filter: F) -> Self {
            self.enabled_sources.extend(self.registry.sources.values().cloned().filter(filter).map(|x| x.id));
            self
        }

        pub fn with_all_products_filtered<F: FnMut(&Good) -> bool>(mut self, filter: F) -> Self {
            self.target_goods.extend(self.registry.goods.values().cloned().filter(filter).map(|x| x.id));
            self
        }

        pub fn for_products(mut self, ids: &[&str]) -> Self {
            self.target_goods.extend(ids.iter().map(|&id| GoodId(id.to_string())));
            self
        }

        pub fn collect(self) -> (Vec<Process>, Vec<Source>) {
            let mut required_goods = self.target_goods.clone();
            let mut queue: Vec<GoodId> = self.target_goods.into_iter().collect();
            let mut seen_processes = HashSet::new();

            while let Some(good_id) = queue.pop() {
                let possible_processes = self.registry.processes.iter()
                    .filter(|(p_id, p)| p_id.0 == good_id.0);

                for proc in possible_processes {
                    let source_ok = self.enabled_sources.contains(&proc.1.source_id);
                    let level_ok = self.registry.goods.get(&good_id)
                        .map(|g| g.level <= self.level)
                        .unwrap_or(false);

                    if source_ok && level_ok && seen_processes.insert(proc.0.clone()) {
                        for (ingredient_id, _) in &proc.1.needs {
                            if required_goods.insert(ingredient_id.clone()) {
                                queue.push(ingredient_id.clone());
                            }
                        }
                    }
                }
            }

            let filtered_processes: Vec<Process> = self.registry.processes.iter()
                .filter(|(_, p)| seen_processes.contains(&p.id))
                .map(|(_, p)| p)
                .cloned()
                .collect();

            let filtered_sources: Vec<Source> = self.enabled_sources.iter()
                .filter_map(|id| self.registry.sources.get(id))
                .cloned()
                .collect();

            (filtered_processes, filtered_sources)
        }
    }

    pub struct DependencyGraph {
        pub inner: Graph<GoodId, u64>,
        pub node_map: HashMap<GoodId, NodeIndex>,
    }

    impl DependencyGraph {
        pub fn with_processes(processes: &[Process]) -> Self {
            let mut graph = Graph::<GoodId, u64>::new();
            let mut node_map = HashMap::<GoodId, NodeIndex>::new();

            for x in processes {
                let product = x.product.0.clone();

                if let None = node_map.get(&product) {
                    node_map.insert(product.clone(), graph.add_node(product.clone()));
                }
            }

            for process in processes {
                let product = process.product.0.clone();

                if let Some(product_ix) = node_map.get(&product) {
                    let product_ix = product_ix.clone();

                    for (need_id, amount) in &process.needs {
                        if let Some(need_ix) = node_map.get(need_id) {
                            graph.add_edge(product_ix.clone(), need_ix.clone(), *amount);
                        } else {
                            let node_ix = graph.add_node(need_id.clone());

                            node_map.insert(need_id.clone(), node_ix.clone());
                            graph.add_edge(product_ix.clone(), node_ix.clone(), *amount);
                        }
                    }
                }
            }

            Self {
                inner: graph,
                node_map,
            }
        }

        pub fn dependencies(&self, node: &GoodId) -> Vec<(&GoodId, u64)> {
            self.node_map.get(node)
                .into_iter()
                .flat_map(|node_ix| {
                    self.inner.edges(*node_ix)
                        .filter(|edge| edge.source() == *node_ix)
                        .filter_map(|e| {
                            self.inner.node_weight(e.target()).map(|id| (id, *e.weight()))
                        })
                })
                .fold(HashMap::new(), |mut acc, node| {
                    *acc.entry(node.0).or_insert(0) += node.1;
                    acc
                })
                .into_iter()
                .collect::<Vec<_>>()
        }

        pub fn dependents(&self, node: &GoodId) -> Vec<(&GoodId, u64)> {
            self.node_map.get(node)
                .into_iter()
                .flat_map(|node_ix| {
                    self.inner.edges(*node_ix)
                        .filter(|edge| edge.target() == *node_ix)
                        .filter_map(|e| {
                            self.inner.node_weight(e.source()).map(|id| (id, *e.weight()))
                        })
                })
                .fold(HashMap::new(), |mut acc, node| {
                    *acc.entry(node.0).or_insert(0) += node.1;
                    acc
                })
                .into_iter()
                .collect::<Vec<_>>()
        }
    }

}

#[derive(Debug, Clone)]
struct RawGood {
    title: String,
    level: String,
    price: String,
    time: String,
    experience: String,
    needs: String,
    source: String,
}

async fn fetch_raw_goods(client: &parsoid::Client) -> anyhow::Result<Vec<RawGood>> {
    let markup = client.get("Goods_List").await?.html().to_string();
    let parsed = scraper::html::Html::parse_document(&markup);

    let row_selector = Selector::parse("tr").unwrap();
    let cell_selector = Selector::parse("td").unwrap();

    let goods = parsed
        .select(&row_selector)
        .filter_map(|row| {
            let cells: Vec<_> = row.select(&cell_selector).collect();
            if cells.len() < 7 { return None; }

            let text = |i: usize| cells[i].text().collect::<Vec<_>>().join(" ").trim().to_string();

            Some(RawGood {
                title: text(0),
                level: text(1),
                price: text(2),
                time: text(3),
                experience: text(4),
                needs: text(5),
                source: text(6),
            })
        })
        .collect();

    Ok(goods)
}

fn create_registry<F>(
    raw_goods: Vec<RawGood>,
    source_time: F
) -> Registry
where
    F : Fn(SourceId) -> Duration,
{
    let res = raw_goods
        .into_iter()
        .flat_map(|result| {
            let source_id = source_id(&result.source).ok()?.1;
            let good_id = domain::GoodId(result.title.clone());

            let good = domain::Good {
                id: good_id.clone(),
                level: result.level.parse::<domain::Level>().ok()?,
                xp: result.experience.parse::<domain::Experience>().ok()?,
                source: source_id.clone(),
                price: result.price.parse().ok()?,
                image_url: None,
            };

            let process = domain::Process {
                id: domain::ProcessId(result.title.clone()),
                source_id: source_id.clone(),
                duration: Duration::from_secs(times_to_seconds(time(&result.time).ok()?.1)),
                goods: vec![good.clone()],
                product: (good_id.clone(), 1),
                needs: needs(&result.needs).ok()?.1
            };

            Some((good, process))
        })
        .collect::<Vec<_>>();

    let machines: HashMap<SourceId, Source> = res
        .iter()
        .cloned()
        .map(|x| (x.1.source_id.clone(), domain::Source {id: x.1.source_id.clone(), capacity: source_time(x.1.source_id)}))
        .collect();

    Registry {
        goods: res.iter().cloned().map(|x| {
            let good = x.0;
            (good.id.clone(), good)
        })
            .collect(),
        processes: res.iter().cloned().map(|(_, p)| (p.id.clone(), p)).collect(),
        sources: machines,
    }
}

fn build_dependency_graph(scenario: Scenario) -> Graph<String, u64> {
    let mut dag = Graph::<String, u64>::default();
    let mut node_map: HashMap<String, NodeIndex> = HashMap::new();

    let scen = scenario.collect();

    println!("{:#?}", scen);

    for x in &scen.0 {
        let product = x.product.0.0.clone();

        if let None = node_map.get(&product) {
            node_map.insert(product.clone(), dag.add_node(product.clone()));
        }
    }

    for x in &scen.0 {
        let product = x.product.0.0.clone();
        let p_idx = *node_map.entry(product.clone()).or_insert_with(|| dag.add_node(product));

        for (need_id, quantity) in &x.needs {
            let need_name = need_id.0.clone();
            let n_idx = *node_map.entry(need_name.clone()).or_insert_with(|| dag.add_node(need_name));
            dag.add_edge(p_idx, n_idx, *quantity);
        }
    }
    dag
}

fn print_production_table(results: Vec<(Job, f64)>, total_value: f64) {
    let mut sorted_res = results;
    sorted_res.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut table = Table::new("{:>} :: {:<} :: [ {:<} ] ( {:<} )");
    table.add_row(Row::new().with_cell("DELTA (s)").with_cell("PRODUCT").with_cell("SOURCE").with_cell("CONSUMING"));
    table.add_heading("----------------------------------------------------------");

    for (job, start_time) in sorted_res {
        let consumes = job.consumes.iter().map(|x| x.0.to_string()).collect::<Vec<_>>();
        table.add_row(
            Row::new()
                .with_cell(format!("{}", start_time.round() as u64))
                .with_cell(job.job_id.0)
                .with_cell(job.machine_id.0)
                .with_cell(format!("{:?}", consumes))
        );
    }

    println!("{}", table);
    println!("---\nTotal Value: {:?}", total_value);
}

fn times_to_seconds(times: Vec<TimeUnit>) -> u64 {
    times.into_iter().fold(0, |acc, tu| match tu {
        TimeUnit::Seconds(t) => acc + t,
        TimeUnit::Minutes(t) => acc + t * 60,
        TimeUnit::Hours(t) => acc + t * 60 * 60,
        TimeUnit::Days(t) => acc + t * 60 * 60 * 24,
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = parsoid::Client::new("https://hayday.fandom.com/rest.php", "parsoid haydata")?;

    let raw_data = fetch_raw_goods(&client).await?;
    let registry = create_registry(raw_data, |_| Duration::from_mins(60*3));

    println!("{:?}", registry.processes);

    let scen = Scenario::new(&registry)
        .limit_level(39)
        .with_all_products_filtered(|x| true)
        .with_all_sources_filtered(|x| x.id != SourceId("Field".to_string())).clone();

    let items = scen.clone().collect();

    let _dag = build_dependency_graph(scen);

    let inventory = HashMap::from_iter(vec![
        (domain::GoodId("Wheat".to_string()), 40),
        (domain::GoodId("Cherry".to_string()), 20),
        (domain::GoodId("Sugarcane".to_string()), 20),
    ]);

    let solution = solvers::planner::solve_production_plan(
        &items.0,
        &items.1,
        &inventory,
        |proc| proc.goods.iter().map(|x| x.price as f64).sum(),
        HashMap::new()
    ).unwrap();

    let deps = DependencyGraph::with_processes(&items.0);
    let jobs = generate_jobs(&registry, &deps, solution.clone());
    let (_, results) = Scheduler::new(jobs).solve()
        .ok_or_else(|| anyhow::anyhow!("Scheduling failed"))?;

    print_production_table(results, solution.total_value);

    Ok(())
}