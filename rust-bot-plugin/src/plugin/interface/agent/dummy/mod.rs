use log::{error, info};
use rand::Rng;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;

use crate::plugin::interface::{Agent, TaskResult};

use crate::plugin::interface::agent::AGENT_STORE;

#[derive(Clone)]
pub struct DummyAgent {
    pub update_interval_in_secs: i64,
    pub retry_delay_in_secs: HashMap<DummyTasks,i64>,
    initial_retry_delay: i64,
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub enum DummyTasks {
    DummyTask,
}
impl Default for DummyAgent {
    fn default() -> Self {
        Self {
            update_interval_in_secs: 3,
            retry_delay_in_secs: HashMap::new(),
            initial_retry_delay: 1,
        }
    }
}

impl DummyAgent {
    async fn try_something(_agent: DummyAgent) -> anyhow::Result<()> {
        info!("now running");

        // get all errors and log them
        // so we understand the time-out error,
        // so we can save a item with information about the time-out
        // so we can each agent have respect the time-out.
        let task_store = AGENT_STORE.clone();
        task_store.error_iter(None).for_each(|(key, error)| {
            error!("error_iter: key: {}, error: {}", key, error);
        });

        let mut rng = rand::thread_rng();
        let action = rng.gen_range(0..2);

        match action {
            0 => Ok(()),                                           // Return as is now
            1 => Err(anyhow::anyhow!("Randomly generated error")), // Return an error
            _ => unreachable!(),
        }
    }

    fn something(&self) -> Pin<Box<dyn Future<Output = TaskResult<DummyTasks>> + Send>> {
        let agent = self.clone();

        Box::pin(async move {
            TaskResult::new(
                DummyTasks::DummyTask,
                DummyAgent::try_something(agent).await,
            )
        })
    }
}

impl Agent for DummyAgent {
    type TaskType = DummyTasks;

    fn get_tasks(
        &self,
        tasks_pending: HashSet<Self::TaskType>,
    ) -> HashMap<
                        Self::TaskType,
                        Pin<Box<dyn Future<Output = TaskResult<Self::TaskType>> + Send>>,
                    >
    {
        let self_clone = self.clone();

        let mut fns = HashMap::new();
        if !tasks_pending.iter().any(|x| x == &DummyTasks::DummyTask) {
            fns.insert(DummyTasks::DummyTask, self_clone.something());
        }
        fns
    }

    fn get_update_interval_in_secs(&self, _task_type: &Self::TaskType) -> i64 {
        self.update_interval_in_secs
    }
    fn get_retry_delay_in_secs(&self, task_type: &Self::TaskType) -> i64 {
        *self.retry_delay_in_secs.get(task_type).unwrap_or(&self.initial_retry_delay)
    }
    fn set_retry_delay_in_secs(&mut self, task_type: &Self::TaskType, retry_delay: i64) {
        self.retry_delay_in_secs.insert(task_type.clone(), retry_delay);
    }

    fn reset_retry_delay(&mut self, task_type: &Self::TaskType) {
        self.set_retry_delay_in_secs(task_type, self.initial_retry_delay);
    }
}
