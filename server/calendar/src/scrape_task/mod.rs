mod main_api_connector;
mod scrape_room_task;
mod statistics;
pub mod tumonline_calendar_connector;

use crate::scrape_task::main_api_connector::get_all_ids;
use crate::scrape_task::scrape_room_task::ScrapeRoomTask;
use crate::scrape_task::statistics::Statistic;
use crate::scrape_task::tumonline_calendar_connector::{Strategy, XMLEvents};
use crate::utils;
use chrono::{DateTime, NaiveDate, Utc};
use diesel::prelude::*;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use log::{info, warn};
use std::time::{Duration, Instant};
use tokio::time::sleep;

pub struct ScrapeTask {
    time_window: chrono::Duration,
    scraping_start: DateTime<Utc>,
}

const CONCURRENT_REQUESTS: usize = 2;
impl ScrapeTask {
    pub fn new(time_window: chrono::Duration) -> Self {
        Self {
            time_window,
            scraping_start: Utc::now(),
        }
    }

    pub async fn scrape_to_db(&self) {
        info!("Starting scraping calendar entries");
        let start_time = Instant::now();

        let mut all_room_ids = get_all_ids().await;
        let entry_cnt = all_room_ids.len();
        let mut time_stats = Statistic::new();
        let mut entry_stats = Statistic::new();

        let mut work_queue = FuturesUnordered::new();
        let start = self.scraping_start - self.time_window / 2;
        while !all_room_ids.is_empty() {
            while work_queue.len() < CONCURRENT_REQUESTS {
                if let Some(room) = all_room_ids.pop() {
                    // sleep to not overload TUMonline.
                    // It is critical for successfully scraping that we are not blocked.
                    sleep(Duration::from_millis(50)).await;

                    work_queue.push(scrape(
                        (room.key.clone(), room.tumonline_room_nr),
                        start.date_naive(),
                        self.time_window,
                    ));
                }
            }
            if let Some(res) = work_queue.next().await {
                entry_stats.push(res.success_cnt as u32);
                // if one of the futures needed to be retried smaller, this would skew the stats a lot
                if !res.retry_smaller_happened {
                    time_stats.push(res.elapsed_time);
                }
            }

            let scraped_entries = entry_cnt - all_room_ids.len();
            if scraped_entries % 30 == 0 {
                let progress = scraped_entries as f32 / entry_cnt as f32 * 100.0;
                let elapsed = start_time.elapsed();
                let time_per_key = elapsed / scraped_entries as u32;
                info!("Scraped {progress:.2}% (avg {time_per_key:.1?}/key, total {elapsed:.1?}) result-{entry_stats:?} in time-{time_stats:.1?}");
            }
        }

        info!(
            "Finished scraping calendar entrys. ({entry_cnt} entries in {:?})",
            start_time.elapsed()
        );
    }

    pub fn delete_stale_results(&self) {
        use crate::schema::calendar::dsl::*;
        let start_time = Instant::now();
        let scrapeinterval = (
            self.scraping_start - self.time_window / 2,
            self.scraping_start + self.time_window / 2,
        );
        let conn = &mut utils::establish_connection();
        diesel::delete(calendar)
            .filter(dtstart.gt(scrapeinterval.0.naive_local()))
            .filter(dtend.le(scrapeinterval.1.naive_local()))
            .filter(last_scrape.le(self.scraping_start.naive_local()))
            .execute(conn)
            .expect("Failed to delete calendar");

        let passed = start_time.elapsed();
        info!(
            "Finished deleting stale results ({time_window} in {passed:?})",
            time_window = self.time_window
        );
    }
}

struct ScrapeResult {
    retry_smaller_happened: bool,
    elapsed_time: Duration,
    success_cnt: usize,
}

async fn scrape(id: (String, i32), from: NaiveDate, duration: chrono::Duration) -> ScrapeResult {
    // request and parse the xml file
    let start_time = Instant::now();
    let mut request_queue = vec![ScrapeRoomTask::new(id, from, duration)];
    let mut success_cnt = 0;
    let mut retry_smaller_happened = false;
    while !request_queue.is_empty() {
        let mut new_request_queue = vec![];
        for task in request_queue {
            let events = XMLEvents::request(task.clone()).await;

            //store the events in the database if successful, otherwise retry
            match events {
                Ok(events) => {
                    success_cnt += events.len();
                    events.store_in_db();
                }
                Err(retry) => match retry {
                    Strategy::NoRetry => {}
                    Strategy::RetrySmaller => {
                        if task.num_days() > 1 {
                            let (t1, t2) = task.split();
                            new_request_queue.push(t1);
                            new_request_queue.push(t2);
                        } else {
                            warn!("The following ScrapeOrder cannot be fulfilled: {task:?}");
                        }
                        retry_smaller_happened = true;
                    }
                },
            };

            // sleep to not overload TUMonline.
            // It is critical for successfully scraping that we are not blocked.
            sleep(Duration::from_millis(50)).await;
        }
        request_queue = new_request_queue;
    }
    ScrapeResult {
        retry_smaller_happened,
        elapsed_time: start_time.elapsed(),
        success_cnt,
    }
}
