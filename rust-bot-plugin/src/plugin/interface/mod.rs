pub mod agent;

use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::future::Future;
use std::hash::Hash;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use chrono::Utc;
use log::{error, info};
use rand::Rng;
use tokio::sync::{Mutex, RwLock};
use tokio::task::{JoinError, JoinSet};
use crate::{CANCELLATION_FLAG, RT, TaskState};
use tokio_util::time::DelayQueue;


pub struct TaskResult<T> {
    pub task_type: T,
    pub task_result: anyhow::Result<()>,
    pub timestamp: i64,
}

impl<T> TaskResult<T> {
    pub fn new(task_type: T, task_result: anyhow::Result<()>) -> Self {
        Self {
            task_type,
            task_result,
            timestamp: Utc::now().timestamp(),
        }
    }
}

struct DelayedResultQueue<T> {
    pub inner: DelayQueue<TaskResult<T>>,
}

impl<T> Future for DelayedResultQueue<T> {
    type Output = TaskResult<T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Poll the delay queue to check for expired tasks
        match Pin::new(&mut self.inner).poll_expired(cx) {
            Poll::Ready(Some(expired_entry)) => {
                // An entry in the delay queue has expired
                let task_result = expired_entry.into_inner();

                // Return the task result
                Poll::Ready(task_result)
            }
            Poll::Ready(None) => {
                // There are no expired tasks yet
                Poll::Pending
            }
            Poll::Pending => {
                // The delay queue is not ready yet
                Poll::Pending
            }
        }
    }
}

pub struct AgentManager<T> where
    T: Clone + Send + Sync + Hash + Eq + Debug + 'static {
    agent: Arc<RwLock<Box<dyn Agent<TaskType = T>>>>,
    delay_queue: Arc<Mutex<DelayedResultQueue<T>>>,
    join_set: Arc<Mutex<JoinSet<TaskResult<T>>>>,
    task_registry: Arc<Mutex<HashMap<T, TaskState>>>,
}

impl <T>AgentManager<T>
    where
    T: Clone + Send + Sync + Hash + Eq + Debug + 'static
    {
    pub fn new(agent: Box<dyn Agent<TaskType = T>>)  -> Self
   {
       AgentManager::<T> {
            agent: Arc::new(RwLock::new(agent)),
            delay_queue: Arc::new(Mutex::new(DelayedResultQueue { inner: DelayQueue::new() })),
            join_set: Arc::new(Mutex::new(JoinSet::new())),
            task_registry: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    pub fn run(&mut self) -> Option<Vec<tokio::task::JoinHandle<()>>>
    {
        let rt_clone = RT.clone();
        let runtime = rt_clone.read().unwrap();

        runtime.as_ref().map(|runtime| {

            CANCELLATION_FLAG.store(false, Ordering::SeqCst);

            let agent = self.agent.clone();
            let delay_queue = self.delay_queue.clone();
            let join_set = self.join_set.clone();
            let task_registry = self.task_registry.clone();


            let handle_delay_queue = runtime.spawn(async move {

                {
                    let fns = agent.read().await.get_tasks(HashSet::new());

                    if fns.len() != 0 {
                        info!("tasks added: {:#?}", fns.keys());

                        let mut tr = task_registry.lock().await;
                        let mut j_s = join_set.lock().await;

                        for (task_type, func) in fns {
                            tr.insert(
                                task_type,
                                TaskState::Pending(process_join_set(&mut j_s, func)),
                            );
                        }
                    }
                }


                while !CANCELLATION_FLAG.load(Ordering::SeqCst) {

                    let mut delay_queue = delay_queue.lock().await;

                    if let Ok(expired) = tokio::time::timeout(Duration::from_millis(100), (&mut *delay_queue)).await {
                        let mut tr = task_registry.lock().await;

                        match expired.task_result {
                            Ok(_) => {
                                tr.insert(
                                    expired.task_type,
                                    TaskState::Resolved(expired.timestamp),
                                );
                            },
                            Err(_) => {
                                tr.insert(
                                    expired.task_type,
                                    TaskState::Failed(expired.timestamp),
                                );;
                            }
                        };

                        let tasks_pending = tr
                            .iter()
                            .filter_map(|(task_type, task_status)| match task_status {
                                TaskState::Pending(_) => Some(task_type.clone()),
                                _ => None
                            })
                            .collect::<HashSet<T>>();

                        let fns = agent.read().await.get_tasks(tasks_pending);
                        if fns.len() != 0 {
                            info!("tasks added: {:#?}", fns.keys());

                            let mut j_s = join_set.lock().await;
                            for (task_type, func) in fns {
                                tr.insert(
                                    task_type,
                                    TaskState::Pending(process_join_set(&mut j_s, func)),
                                );
                            }
                        }
                    }

                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                let mut j_s = join_set.lock().await;
                j_s.shutdown().await;
            });


            let agent = self.agent.clone();
            let delay_queue = self.delay_queue.clone();
            let join_set = self.join_set.clone();
            let task_registry = self.task_registry.clone();

            let handle_tasks = runtime.spawn(async move {

                while !CANCELLATION_FLAG.load(Ordering::SeqCst) {

                    // wait for one task to complete!
                    let mut j_s = join_set.lock().await;

                    for each in poll_completed_tasks(&mut j_s).await {
                        match each {
                            Ok(task_result) => match task_result.task_result {
                                Ok(_) => {
                                    info!("task resolved: {:?}", &task_result.task_type);
                                    let mut tmp_agent = agent.write().await;
                                    tmp_agent.reset_retry_delay(&task_result.task_type);
                                    let mut tmp_delay_queue = delay_queue.lock().await;

                                    let delay_duration = Duration::from_secs(tmp_agent.get_update_interval_in_secs(&task_result.task_type) as u64);
                                    tmp_delay_queue.inner.insert(task_result, delay_duration);
                                }
                                Err(ref err) => {
                                    error!("task failed: {:?}", (&task_result.task_type, err));
                                    let mut tmp_agent = agent.write().await;
                                    tmp_agent.exponential_backoff_with_jitter(&task_result.task_type);
                                    let mut tmp_delay_queue = delay_queue.lock().await;
                                    let delay_duration = Duration::from_secs(tmp_agent.get_retry_delay_in_secs(&task_result.task_type) as u64);
                                    tmp_delay_queue.inner.insert(task_result, delay_duration);
                                }
                            },
                            Err(err) => {
                                let task_id = err.id();
                                let mut tr = task_registry.lock().await;

                                let _ = tr
                                    .iter_mut()
                                    .filter(|(_, task_state)| {
                                        if let TaskState::Pending(id) = task_state {
                                            &task_id == id
                                        } else {
                                            false
                                        }
                                    })
                                    .map(|(task_type, task_state)| {
                                        let timestamp = Utc::now().timestamp();
                                        if err.is_cancelled() {
                                            error!("task cancelled: {:?}", (&task_type, &err));
                                            *task_state = TaskState::Cancelled(timestamp);
                                        } else if err.is_panic() {
                                            error!("task panicked: {:?}", (&task_type, &err));
                                            *task_state = TaskState::Panicked(timestamp);
                                        } else {
                                            unreachable!()
                                        }
                                    });
                            }
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                let mut j_s = join_set.lock().await;
                j_s.shutdown().await;
            });
            vec![handle_delay_queue,handle_tasks]
        })
    }
}


pub trait Agent: Send + Sync {
    type TaskType: Clone + Hash + Eq + Debug + Send + 'static;
    fn get_tasks(
        &self,
        tasks_pending: HashSet<Self::TaskType>,
    ) -> HashMap<
        Self::TaskType,
        Pin<Box<dyn Future<Output = TaskResult<Self::TaskType>> + Send>>
    >;
    fn get_update_interval_in_secs(&self, task_type: &Self::TaskType) -> i64;
    fn get_retry_delay_in_secs(&self, task_type: &Self::TaskType) -> i64;
    fn set_retry_delay_in_secs(&mut self, task_type: &Self::TaskType, retry_interval: i64);

    fn exponential_backoff_with_jitter(&mut self, task_type: &Self::TaskType) {
        let current_retry_delay =self.get_retry_delay_in_secs(task_type);
        let max_interval = 60 * 5;
        let backoff_factor = 2;
        let jitter = rand::thread_rng().gen_range(0..=current_retry_delay / 2);

        let val = (current_retry_delay * backoff_factor + jitter).min(max_interval);
        self.set_retry_delay_in_secs(task_type,val);
    }

    fn reset_retry_delay(&mut self, task_type: &Self::TaskType);




    // clear all data from store
    // export all data from store
    // import all data from file to store

    // task meta data / process information might be written into temporary_sled!
}

pub fn process_join_set<T, F>(join_set: &mut JoinSet<TaskResult<T>>, f: F) -> tokio::task::Id
where
    T: Clone + Send + Sync + Hash + Eq + Debug + 'static,
    F: Future<Output = TaskResult<T>> + Send + 'static,
{
    join_set.spawn(async move {
        f.await
    }).id()
}

pub fn poll_completed_tasks<T>(
    join_set: &mut JoinSet<T>,
) -> Pin<Box<dyn Future<Output = Vec<Result<T, JoinError>>> + Send + '_>>
where
    T: Send + 'static,
{
    Box::pin(async move {
        let mut completed: Vec<Result<T, JoinError>> = Vec::new();

        // The following removes all completed tasks from the set.
        // Unresolved tasks are unaffected.
        for _ in 0..join_set.len() {
            let result =
                tokio::time::timeout(std::time::Duration::from_millis(0), join_set.join_next())
                    .await;
            // join_set.join_next()
            // If this returns `Poll::Ready(Some(_))`, then the task that completed is removed from the set.
            match result {
                Ok(Some(res)) => {
                    completed.push(res);
                }
                Err(_) | Ok(None) => {
                    // timeout returned an error (currently all tasks pending) or join_set empty
                    break;
                }
            };
        }
        completed
    })
}
