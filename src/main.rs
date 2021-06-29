#[macro_use]
extern crate schemamama;
extern crate postgres;
extern crate schemamama_postgres;
#[macro_use]
extern crate log;
extern crate iron;
extern crate log4rs;
extern crate logger;
extern crate mustache;
extern crate persistent;
extern crate r2d2;
extern crate r2d2_postgres;
extern crate router;
#[macro_use]
extern crate mime;
#[macro_use]
extern crate lazy_static;
extern crate params;
extern crate regex;
extern crate reqwest;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate base64;
extern crate serde_yaml;
#[macro_use]
extern crate error_chain;

use db::PostgresConnection;
use iron::modifiers::RedirectRaw;
use iron::prelude::*;
use iron::status;
use logger::Logger;
use mustache::MapBuilder;
use params::{Params, Value};
use persistent::Read as PRead;
use router::Router;
use std::env;
use std::fs::File;
use std::str::FromStr;

#[macro_use]
mod db;
mod errors;
mod schema;
use errors::*;
mod common;
use common::*;

#[derive(Debug, Serialize, Deserialize)]
struct TwitterConfig {
    key: String,
    secret: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Config {
    twitter: TwitterConfig,
}

#[derive(Debug, Deserialize)]
struct BearerToken {
    token_type: String,
    access_token: String,
}

lazy_static! {
    static ref CONFIG: Config = {
        if env::var("DYNO").is_ok() {
            // Heroku, so assume environment variables
            Config {
                twitter: TwitterConfig {
                    key: env::var("TWITTER_KEY")
                        .expect("Need TWITTER_KEY environment variable"),
                    secret: env::var("TWITTER_SECRET")
                        .expect("Need TWITTER_SECRET environment variable"),
                }
            }
        }
        else {
            // Local, so assume config file
            let f = File::open("config.yaml").expect("Need config.yaml");
            serde_yaml::from_reader(f).unwrap()
        }
    };
    static ref TOKEN: String = {
        let client = reqwest::Client::new();
        let mut res = client.post("https://api.twitter.com/oauth2/token")
            .basic_auth(CONFIG.twitter.key.clone(), Some(CONFIG.twitter.secret.clone()))
            .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body("grant_type=client_credentials")
            .send().unwrap();
        let content: BearerToken = res.json().unwrap();
        res.error_for_status().unwrap();
        content.access_token
    };
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct TwitterUser {
    id: i64,
    screen_name: String,
    name: String,
    profile_image_url: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct Tweet {
    id: i64,
    user: TwitterUser,
    text: String,
    in_reply_to_status_id: Option<i64>,
    in_reply_to_user_id: Option<i64>,

    // we fill this in from OEmbed, not the original JSON
    #[serde(default)]
    html: String,
}

#[derive(Deserialize, Debug)]
struct TwitterError {
    code: u16,
    message: String,
}

#[derive(Deserialize, Debug)]
struct TwitterErrors {
    errors: Vec<TwitterError>,
}

#[derive(Deserialize, Debug)]
struct SearchResults {
    statuses: Vec<Tweet>,
}

#[derive(Deserialize, Debug)]
struct OEmbed {
    html: String,
}

fn get_user_from_db(conn: &db::PostgresConnection, user_id: i64) -> TwitterUser {
    let users = &conn
        .query(
            concat!(
                "SELECT name, profile_image_url, username ",
                "FROM twitter_user WHERE id = $1"
            ),
            &[&user_id],
        )
        .unwrap();
    let user = users.get(0);
    TwitterUser {
        id: user_id,
        name: user.get(0),
        profile_image_url: user.get(1),
        screen_name: user.get(2),
    }
}

fn store_tweet(conn: &db::PostgresConnection, t: &Tweet) {
    let rows = &conn
        .query("SELECT 1 FROM twitter_user WHERE id = $1", &[&t.user.id])
        .unwrap();
    if rows.is_empty() {
        conn.execute(
            concat!(
                "INSERT INTO twitter_user ",
                "(id, name, profile_image_url, username) VALUES ($1,$2,$3,$4)"
            ),
            &[
                &t.user.id,
                &t.user.name,
                &t.user.profile_image_url,
                &t.user.screen_name,
            ],
        )
        .unwrap();
    }
    let client = reqwest::Client::new();
    let mut content = client
        .get(&format!(
            concat!(
                "https://publish.twitter.com/oembed?url=https://twitter.com/{}/status/{}&",
                "hide_thread=true&omit_script=true&dnt=true"
            ),
            &t.user.screen_name, &t.id
        ))
        .send()
        .unwrap();
    let oembed: OEmbed = content.json().expect("valid oembed data");
    conn.execute(
        concat!(
            "INSERT INTO tweet ",
            "(id, user_id, text, in_reply_to_status_id, in_reply_to_user_id, html) ",
            "VALUES ($1,$2,$3,$4,$5,$6)"
        ),
        &[
            &t.id,
            &t.user.id,
            &t.text,
            &t.in_reply_to_status_id,
            &t.in_reply_to_user_id,
            &oembed.html,
        ],
    )
    .unwrap();
}

fn get_tweet(conn: &db::PostgresConnection, name: &str, id: i64) -> Result<Tweet> {
    let tweets = &conn
        .query(
            concat!(
                "SELECT user_id, text, in_reply_to_status_id, ",
                "in_reply_to_user_id, html ",
                "FROM tweet WHERE id = $1"
            ),
            &[&id],
        )
        .unwrap();
    if !tweets.is_empty() {
        let tweet = tweets.get(0);
        return Ok(Tweet {
            id,
            user: get_user_from_db(conn, tweet.get(0)),
            text: tweet.get(1),
            in_reply_to_status_id: tweet.get(2),
            in_reply_to_user_id: tweet.get(3),
            html: tweet.get(4),
        });
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::RedirectPolicy::none())
        .build()
        .unwrap();
    let mut res = client
        .get(&format!(
            "https://api.twitter.com/1.1/statuses/show.json?id={}",
            id
        ))
        .bearer_auth(TOKEN.clone())
        .send()
        .unwrap();
    if res.status().is_client_error() {
        let err: TwitterErrors = res.json().unwrap();
        warn!("Error: {:?}", err);
        let first = err.errors.first().unwrap();
        if first.code == 144 {
            Err(ErrorKind::NoSuchTweet(id).into())
        } else {
            Err(ErrorKind::OtherTwitterError(
                first.code,
                first.message.clone(),
                format!("https://twitter.com/{}/status/{}", name, id),
            )
            .into())
        }
    } else {
        let t: Tweet = res.json()?;
        store_tweet(conn, &t);
        Ok(t)
    }
}

fn get_tweets(conn: &PostgresConnection, name: &str, id: i64, future_tweets: bool) -> Result<Vec<Tweet>> {
    let mut tweets: Vec<Tweet> = Vec::new();
    let mut current_id = id;
    loop {
        let t = get_tweet(&conn, name, current_id)?;
        let next_id: Option<i64> = if t.in_reply_to_user_id.unwrap_or(-1) == t.user.id {
            t.in_reply_to_status_id
        } else {
            None
        };
        tweets.insert(0, t);
        if next_id.is_none() {
            break;
        }
        current_id = next_id.unwrap();
    }
    if future_tweets {
        let client = reqwest::Client::new();
        let mut current = {
            let last = tweets.last();
            last.unwrap().clone()
        };
        loop {
            let tweets_query = &conn
                .query(
                    concat!(
                        "SELECT id, user_id, text, in_reply_to_status_id,",
                        "in_reply_to_user_id, html FROM tweet WHERE in_reply_to_status_id = $1"
                    ),
                    &[&current.id],
                )
                .unwrap();
            if !tweets_query.is_empty() {
                let tweet = tweets_query.get(0);
                let t = Tweet {
                    id: tweet.get(0),
                    user: get_user_from_db(conn, tweet.get(1)),
                    text: tweet.get(2),
                    in_reply_to_status_id: tweet.get(3),
                    in_reply_to_user_id: tweet.get(4),
                    html: tweet.get(5),
                };
                current = t.clone();
                tweets.push(t);
                continue;
            }

            let mut res = client
                .get(&format!(
                    concat!(
                        "https://api.twitter.com/1.1/search/tweets.json?",
                        "q=to%3A{name}%20from%3A{name}&since_id={id}&include_entities=false&count=100"
                    ),
                    name = current.user.screen_name,
                    id = current.id
                ))
                .bearer_auth(TOKEN.clone())
                .send()
                .unwrap();
            let future_tweets: SearchResults = res.json().unwrap();
            let mut found_extra = false;
            for t in future_tweets.statuses {
                if t.in_reply_to_status_id.unwrap_or(-1) == current.id {
                    store_tweet(conn, &t);
                    current = t.clone();
                    tweets.push(t);
                    found_extra = true;
                }
            }
            if !found_extra {
                break;
            }
        }
    }
    Ok(tweets)
}

pub fn new_tweet(req: &mut Request) -> IronResult<Response> {
    let conn = get_pg_connection!(&req);
    let map = req.get_ref::<Params>().unwrap();
    let find_url = map.find(&["twitter_url"]);
    if find_url.is_none() {
        return Ok(Response::with(status::NotFound));
    }
    let url_value = find_url.unwrap();
    let re = regex::Regex::new(r"twitter.com/([^/]+)/status/(\d+)").unwrap();
    if let Value::String(ref url) = *url_value {
        let raw_caps = re.captures(url);
        if raw_caps.is_none() {
            return Ok(Response::with(status::BadRequest));
        }
        let caps = raw_caps.unwrap();
        let id = i64::from_str(caps.get(2).unwrap().as_str()).unwrap();
        let name = caps.get(1).unwrap().as_str();
        let tweets = get_tweets(&conn, name, id, true)?;
        return Ok(Response::with((
            status::Found,
            RedirectRaw(format!("/tweet/{}/{}", name, tweets.last().unwrap().id)),
        )));
    } else {
        unimplemented!();
    }
}

pub fn tweet(req: &mut Request) -> IronResult<Response> {
    let conn = get_pg_connection!(&req);
    let router = req.extensions.get::<Router>().unwrap();
    let tweet_id: i64 = i64::from_str(router.find("tweet_id").unwrap()).unwrap();
    let name = router.find("name").unwrap_or("");
    let tweets = get_tweets(&conn, name, tweet_id, false)?;
    let data = MapBuilder::new()
        .insert("tweets", &tweets)
        .expect("inserting tweets works")
        .insert_str("title", format!("Gasconade - {}", tweets[0].user.name))
        .build();
    Ok(Response::with((
        mime!(Text / Html),
        status::Ok,
        render_to_response("resources/templates/tweet.mustache", &data),
    )))
}

pub fn index(req: &mut Request) -> IronResult<Response> {
    let map = req.get_ref::<Params>().unwrap();
    let error = map.find(&["error"]);
    let data = MapBuilder::new()
        .insert_str("title", "Gasconade")
        .insert_str(
            "error",
            error
                .map(|v| {
                    if let params::Value::String(ref s) = *v {
                        s.to_owned()
                    } else {
                        String::default()
                    }
                })
                .unwrap_or_default(),
        )
        .build();
    Ok(Response::with((
        mime!(Text / Html),
        status::Ok,
        render_to_response("resources/templates/index.mustache", &data),
    )))
}

fn get_server_port() -> u16 {
    env::var("PORT")
        .unwrap_or_else(|_| "8000".to_string())
        .parse()
        .unwrap()
}

fn main() {
    log4rs::init_file("log.yaml", Default::default()).unwrap();
    let db_url: &str = &env::var("DATABASE_URL").expect("Needed DATABASE_URL");
    let pool = db::get_pool(db_url);
    let conn = pool.get().unwrap();
    schema::up(&conn).unwrap();
    let (logger_before, logger_after) = Logger::new(None);
    let mut router = Router::new();
    router.get("/", index, "index");
    router.post("/tweet", new_tweet, "query tweet");
    router.get("/tweet/:name/:tweet_id", tweet, "tweet_with_name");
    router.get("/tweet/:tweet_id", tweet, "tweet");
    let mut chain = Chain::new(router);
    chain.link_before(logger_before);
    chain.link_after(logger_after);
    chain.link(PRead::<db::PostgresDB>::both(pool));
    info!("Gasconade booted");
    info!("Token is {:?}", *TOKEN);
    Iron::new(chain).http(("0.0.0.0", get_server_port())).unwrap();
}
