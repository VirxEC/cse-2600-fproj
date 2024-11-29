use crate::consts::{API_URL, COUNT, INDEX_DIR, PLAYLIST, RANKS, SEASON};
use reqwest::{Client, Response};
use serde_json::Value;
use std::{collections::HashMap, path::Path, time::Duration};
use tokio::{
    fs,
    time::{interval, sleep, MissedTickBehavior},
};

pub async fn get_url_with_auth(client: &Client, url: &str, token: &str) -> Option<Response> {
    client
        .get(url)
        .header("Authorization", token)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()
}

pub async fn replay_auto_timer(client: Client, token: &str) {
    // we are limited to 2 api calls per second,
    // and 500 api calls per hour
    let mut interval = interval(Duration::from_secs_f32(0.6));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    // get the current hour of the day on the clock
    let mut num_requests = 0;

    loop {
        let mut skip_wait = false;
        let mut wait_minute = false;

        for rank in RANKS {
            if !skip_wait {
                interval.tick().await;
            }

            skip_wait = false;

            println!("# of requests made: {num_requests}");

            if wait_minute {
                println!("Got an error, waiting a minute to continue");
                sleep(Duration::from_secs(60)).await;
                num_requests = 0;
                wait_minute = false;
            }

            let num_processed = fs::read_to_string(format!("{INDEX_DIR}/{rank}/num_processed.txt"))
                .await
                .unwrap()
                .parse::<u64>()
                .unwrap();

            // there are 200 replays per index
            let index_num = num_processed / 200;
            let replay_num = num_processed % 200;

            let index_path_str = format!("{INDEX_DIR}/{rank}/{index_num}.json");
            let index_path = Path::new(&index_path_str);

            let index: HashMap<String, Value> = if index_path.exists() {
                serde_json::from_str(&fs::read_to_string(index_path).await.unwrap())
                    .expect("Failed to parse index file")
            } else {
                // get the previous index file
                let prev_index_num = index_num - 1;
                let prev_index =
                    fs::read_to_string(format!("{INDEX_DIR}/{rank}/{prev_index_num}.json"))
                        .await
                        .unwrap();

                // parse as json and check if "next" key exists
                let prev_index_json: serde_json::Value = serde_json::from_str(&prev_index).unwrap();
                let Some(next_url) = prev_index_json.get("next") else {
                    continue;
                };

                // the api incorrectly returns the next url with a blank max rank, resulting in too many ranks being included
                let fixed_url = next_url
                    .as_str()
                    .unwrap()
                    .replace("max-rank=&", &format!("max-rank={rank}&"));

                // get the next index file & save it
                println!("Downloading next index file for {rank}##{index_num}: {fixed_url}");
                let Some(next_index_req) = get_url_with_auth(&client, &fixed_url, token).await
                else {
                    println!("Failed to retrieve next index file - current number of requests since last wait: {num_requests}");
                    continue;
                };

                let next_index = next_index_req.text().await.unwrap();

                // ensure the response isn't a 200 with an error message
                if next_index.contains(r#""error":"Too many requests""#) {
                    println!("Failed to retrieve next index file - current number of requests since last wait: {num_requests}");
                    continue;
                }

                fs::write(index_path, &next_index).await.unwrap();
                serde_json::from_str(&next_index).expect("Failed to parse index file")
            };

            let replay_count = index
                .get("count")
                .expect("Failed to get count key")
                .as_u64()
                .unwrap();

            if replay_count == num_processed {
                println!("All replays for {rank} have been processed");
                skip_wait = true;
                continue;
            }

            // get the download link
            let replays = index
                .get("list")
                .expect("Failed to get list key")
                .as_array()
                .unwrap();

            let Some(replay_url) = replays.get(replay_num as usize).map(|item| {
                item.as_object()
                    .unwrap()
                    .get("link")
                    .unwrap()
                    .as_str()
                    .unwrap()
            }) else {
                println!("Failed to get replay link for {rank}##{index_num}+{replay_num}");

                // incr num_processed
                fs::write(
                    format!("{INDEX_DIR}/{rank}/num_processed.txt"),
                    format!("{}", num_processed + 1),
                )
                .await
                .unwrap();

                continue;
            };

            println!("Downloading replay file: {replay_url}");
            let Some(replay_req) = get_url_with_auth(&client, replay_url, token).await else {
                println!("Failed to retrieve replay file for {rank}##{index_num}+{replay_num} - no internet - current number of requests since last wait: {num_requests}");
                wait_minute = true;
                continue;
            };

            num_requests += 1;

            if replay_req.status().as_u16() != 200 {
                println!("Failed to retrieve replay file for {rank}##{index_num}+{replay_num} - non-200 error code - current number of requests since last wait: {num_requests}");
                wait_minute = true;
                continue;
            }

            let Ok(replay_json) = replay_req.text().await else {
                println!("Failed to read replay file for {rank}##{index_num}+{replay_num} - current number of requests since last wait: {num_requests}");
                wait_minute = true;
                continue;
            };

            // ensure the response isn't a 200 with an error message
            if replay_json.contains(r#""error":"Too many requests""#) {
                println!("Failed to retrieve replay file for {rank}##{index_num}+{replay_num} - rate limit - current number of requests since last wait: {num_requests}");
                wait_minute = true;
                continue;
            }

            // write parsed replay to file
            fs::create_dir_all(format!("{INDEX_DIR}/{rank}/parsed"))
                .await
                .unwrap();
            fs::write(
                format!("{INDEX_DIR}/{rank}/parsed/{}.json", num_processed),
                replay_json,
            )
            .await
            .unwrap();

            // incr num_processed
            fs::write(
                format!("{INDEX_DIR}/{rank}/num_processed.txt"),
                format!("{}", num_processed + 1),
            )
            .await
            .unwrap();
        }
    }
}

pub async fn get_initial_index(client: Client, token: &str) {
    // The authenticate an API call, you must pass the API token as the value of the Authorization header:

    // Authorization: $token
    for rank in RANKS {
        println!("Downloading initial replay index for {rank}");

        let url = format!("{API_URL}?playlist={PLAYLIST}&season={SEASON}&min-rank={rank}&max-rank={rank}&count={COUNT}");
        let body = get_url_with_auth(&client, &url, token)
            .await
            .unwrap()
            .text()
            .await
            .expect("Failed to read response body");

        // setup index directory
        fs::create_dir_all(format!("{INDEX_DIR}/{rank}"))
            .await
            .unwrap();
        fs::write(format!("{INDEX_DIR}/{rank}/0.json"), body)
            .await
            .unwrap();

        fs::write(format!("{INDEX_DIR}/{rank}/num_processed.txt"), "0")
            .await
            .unwrap();
    }

    // Example URI
    // GET https://ballchasing.com/api/replays
    // URI Parameters
    // playlist
    //     enum (optional)

    //     filter replays by one or more playlists

    //     Choices: unranked-duels unranked-doubles unranked-standard unranked-chaos private season offline ranked-duels ranked-doubles ranked-solo-standard ranked-standard snowday rocketlabs hoops rumble tournament dropshot ranked-hoops ranked-rumble ranked-dropshot ranked-snowday dropshot-rumble heatseeker
    // season
    //     string (optional)

    //     filter replays by season. Must be a number between 1 and 14 (for old seasons) or f1, f2, … for the new free to play seasons
    // min-rank
    //     enum (optional)

    //     filter your replays based on players minimum rank

    //     Choices: unranked bronze-1 bronze-2 bronze-3 silver-1 silver-2 silver-3 gold-1 gold-2 gold-3 platinum-1 platinum-2 platinum-3 diamond-1 diamond-2 diamond-3 champion-1 champion-2 champion-3 grand-champion
    // max-rank
    //     enum (optional)

    //     filter your replays based on players maximum rank

    //     Choices: unranked bronze-1 bronze-2 bronze-3 silver-1 silver-2 silver-3 gold-1 gold-2 gold-3 platinum-1 platinum-2 platinum-3 diamond-1 diamond-2 diamond-3 champion-1 champion-2 champion-3 grand-champion
    // count
    //     numeric (optional) Default: 150

    //     returns at most count replays. Between 1 and 200
}
