mod solvers;
mod parsers;

use crate::domain::{DependencyGraph, Registry, Scenario, Source, SourceId};
use crate::parsers::{needs, source_id, time, TimeUnit};
use crate::solvers::scheduler::{generate_jobs, Scheduler};
use mediawiki::prelude::*;
use nom::Parser;
use parsoid::prelude::*;
use petgraph::prelude::*;
use scraper::Selector;
use std::collections::HashMap;
use std::time::Duration;
use tabular::{Row, Table};
use url::Url;

pub mod domain {
    use good_lp::{constraint, default_solver, variable, variables, Expression, Solution, SolverModel};
    use petgraph::graph::{IndexType, NodeIndex};
    use petgraph::prelude::EdgeRef;
    use petgraph::visit::Walker;
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
        registry: &'a Registry,
        level: u64,
        enabled_sources: HashSet<SourceId>,
        target_goods: HashSet<GoodId>,
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

async fn fetch_category_members(api: &mediawiki::Api, category: &str) -> Option<Vec<String>> {
    let query = ActionApiList::categorymembers()
        .cmtitle(format!("Category:{}", category))
        .cmlimit(200)
        .run(&api)
        .await
        .ok()?;

    query["query"]["categorymembers"].as_array().map(|arr| {
        arr.iter()
            .flat_map(|val| {
                let b = val["title"].as_str();
                b.map(String::from)
            })
            .collect::<Vec<_>>()
    })
}

async fn fetch_infobox<T, F>(
    api: &mediawiki::Api,
    client: ParsoidClient,
    titles: Vec<String>,
    extractor: F,
) -> Option<Vec<T>>
where
    F: Fn(String, Template) -> Option<T> + Clone,
{
    let tasks = titles.into_iter().map(|val| {
        let client = client.clone();
        let extractor = extractor.clone();
        async move {
            let name = val.to_string();

            let code = client.get(&name).await.ok()?;
            let template = code
                .into_mutable()
                .filter_templates()
                .ok()?
                .into_iter()
                .find(|t| t.name() == "Template:Infobox")?;

            extractor(name, template)
        }
    });

    let results: Vec<Option<T>> = futures::future::join_all(tasks).await;

    Some(results.into_iter().flatten().collect())
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
    let api = Api::new("https://hayday.fandom.com/api.php").await?;
    let client = parsoid::Client::new("https://hayday.fandom.com/rest.php", "parsoid haydata")?;

    let markup = client.get("Goods_List").await?.html().to_string();

    let parsed = scraper::html::Html::parse_document(&markup);

    let row_selector = Selector::parse("tr").unwrap();
    let cell_selector = Selector::parse("td").unwrap();

    let res = parsed
        .select(&row_selector)
        .flat_map(|row| {
            let cells: Vec<_> = row.select(&cell_selector).collect();

            if (cells.is_empty()) {
                return None;
            }

            let name = cells[0]
                .text()
                .collect::<Vec<_>>()
                .join(" ")
                .trim()
                .to_string();
            let level = cells[1].text().collect::<Vec<_>>().join(" ").to_string();
            let price = cells[2].text().collect::<Vec<_>>().join(" ").to_string();
            let time = cells[3].text().collect::<Vec<_>>().join(" ").to_string();
            let exp = cells[4].text().collect::<Vec<_>>().join(" ").to_string();
            let needs = cells[5].text().collect::<Vec<_>>().join(" ").to_string();
            let sources = cells[6].text().collect::<Vec<_>>().join(" ").to_string();

            Some(RawGood {
                title: name,
                level,
                price,
                time,
                experience: exp,
                needs,
                source: sources,
            })
        })
        .collect::<Vec<_>>();

    let mut good_image_uri: HashMap<String, Url> = HashMap::new();

    let res = res.clone()
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
                image_url: good_image_uri.get(&result.title.clone()).cloned(),
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
        .map(|x| (x.1.source_id.clone(), domain::Source {id: x.1.source_id, capacity: Duration::from_mins(60*6)}))
        .collect();

    let registry = Registry {
        goods: res.iter().cloned().map(|x| {
            let good = x.0;

            (good.id.clone(), good)
        })
            .collect(),
        processes: res.iter().cloned().map(|(_, p)| (p.id.clone(), p)).collect(),
        sources: machines,
    };

    let scen = Scenario::new(&registry)
        .limit_level(170)
        .with_all_sources_filtered(|x| x.id != SourceId("Field".to_string()))
        .with_all_products_filtered(|x| true)
        .collect();

    let mut dag = Graph::<String, u64>::default();
    let mut node_map: HashMap<String, NodeIndex> = HashMap::new();

    for x in &scen.0 {
        let product = x.product.0.0.clone();

        if let None = node_map.get(&product) {
            node_map.insert(product.clone(), dag.add_node(product.clone()));
        }
    }

    for x in &scen.0 {
        let product = x.product.0.0.clone();

        if let Some(product_ix) = node_map.get(&product) {
            let product_ix = product_ix.clone();

            for y in &x.needs {
                let need = y.0.0.clone();

                if let Some(need_ix) = node_map.get(&need) {
                    dag.add_edge(product_ix.clone(), need_ix.clone(),  y.1);
                } else {
                    let node_ix = dag.add_node(need.clone());

                    node_map.insert(need.clone(), node_ix.clone());
                    dag.add_edge(product_ix.clone(), node_ix.clone(), y.1);
                }
            }
        }
    }

    let solution = solvers::planner::solve_production_plan(
        &scen.0[..],
        &scen.1[..],
        &HashMap::from_iter(vec![
            (domain::GoodId("corn".to_string()), 10),
            (domain::GoodId("Wheat".to_string()), 40),
            (domain::GoodId("Milk".to_string()), 30),
            (domain::GoodId("Sugarcane".to_string()), 30),
            (domain::GoodId("Cherry".to_string()), 20),
        ]),
        |proc| { proc.goods.iter().map(|x| x.price as f64).sum() },
        HashMap::new()
    );

    println!("{:#?}", solution.clone().map(|xx| (xx.total_value, xx.counts.into_iter().map(|yy| (yy.0.0, yy.1)).filter(|z| z.1 > 0).collect::<Vec<_>>())));

    let deps = DependencyGraph::with_processes(&scen.0[..]);

    let jobs = generate_jobs(&registry, &deps, solution.clone().unwrap());

    let schedule = Scheduler::new(jobs).solve();

    let mut ress = schedule.unwrap().1;

    ress.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

    let mut table = Table::new("{:>} :: {:<} :: [ {:<} ] ( {:<} )");

    table.add_row(Row::new().with_cell("DELTA (secs)").with_cell("PRODUCT").with_cell("SOURCE").with_cell("CONSUMING"));
    table.add_heading("----------------------------------------------------------");

    for (asd, ada) in ress {
        table.add_row(
            Row::new()
                .with_cell(format!("{} (s)", ada.round() as u64)).with_cell(asd.job_id.0).with_cell(asd.machine_id.0).with_cell(format!("{:?}", asd.consumes.iter().map(|x| x.0.to_string()).collect::<Vec<_>>()))
        );
    }

    println!("{}", table);
    println!("---");
    println!("{:?}", &solution.unwrap().total_value);

    return Ok(());
}
