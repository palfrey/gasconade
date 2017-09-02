#[macro_use]
extern crate schemamama;
extern crate schemamama_postgres;
extern crate postgres;
#[macro_use]
extern crate log;
extern crate log4rs;
extern crate iron;
extern crate router;
extern crate logger;
extern crate r2d2;
extern crate r2d2_postgres;
extern crate persistent;
extern crate mustache;
#[macro_use]
extern crate mime;
#[macro_use]
extern crate lazy_static;
extern crate regex;
extern crate params;
extern crate reqwest;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_yaml;
extern crate base64;

use iron::prelude::*;
use iron::status;
use router::Router;
use logger::Logger;
use std::env;
use persistent::Read as PRead;
use mustache::MapBuilder;
use params::{Params, Value};
use std::fs::File;
use reqwest::header::{Authorization, Bearer};
use std::str::FromStr;
use iron::modifiers::RedirectRaw;

#[macro_use]
mod db;
mod schema;

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
        let f = File::open("config.yaml").unwrap();
        serde_yaml::from_reader(f).unwrap()
    };
    static ref TOKEN: String = {
        let client = reqwest::Client::new().unwrap();
        let mut res = client.post("https://api.twitter.com/oauth2/token").unwrap()
            .basic_auth(CONFIG.twitter.key.clone(), Some(CONFIG.twitter.secret.clone()))
            .header(reqwest::header::ContentType::form_url_encoded())
            .body("grant_type=client_credentials")
            .send().unwrap();
        let content: BearerToken = res.json().unwrap();
        res.error_for_status().unwrap();
        content.access_token
    };
}

pub fn render_to_response(path: &str, data: &mustache::Data) -> Vec<u8> {
    let template = mustache::compile_path(path).expect(&format!("working template for {}", path));
    let mut buffer: Vec<u8> = vec![];
    template.render_data(&mut buffer, data).unwrap();
    return buffer;
}

#[derive(Deserialize, Debug)]
struct TwitterUser {
    id: i64,
    name: String,
    profile_image_url: String,
}

#[derive(Deserialize, Debug)]
struct Tweet {
    id: i64,
    user: TwitterUser,
    text: String,
    in_reply_to_status_id: Option<i64>,
    in_reply_to_user_id: Option<i64>,
}

fn get_tweet(conn: &db::PostgresConnection, id: i64) -> Tweet {
    let tweets = &conn.query("
    SELECT user_id, text, in_reply_to_status_id, in_reply_to_user_id FROM tweet WHERE id = $1",
                             &[&id])
                      .unwrap();
    if !tweets.is_empty() {
        let tweet = tweets.get(0);
        let user_id: i64 = tweet.get(0);
        let users = &conn.query("SELECT name, profile_image_url FROM twitter_user WHERE id = $1",
                                &[&user_id])
                         .unwrap();
        let user = users.get(0);
        return Tweet {
                   id: id,
                   user: TwitterUser {
                       id: user_id,
                       name: user.get(0),
                       profile_image_url: user.get(1),
                   },
                   text: tweet.get(1),
                   in_reply_to_status_id: tweet.get(2),
                   in_reply_to_user_id: tweet.get(3),
               };
    }
    let client = reqwest::Client::new().unwrap();
    let mut res = client.get(&format!("https://api.twitter.com/1.1/statuses/show.json?id={}", id))
        .unwrap()
        .header(Authorization(Bearer { token: TOKEN.clone() }))
        .send()
        .unwrap();
    let t: Tweet = res.json().unwrap();
    let rows = &conn.query("SELECT 1 FROM twitter_user WHERE id = $1", &[&t.user.id]).unwrap();
    if rows.is_empty() {
        conn.execute("INSERT INTO twitter_user (id, name, profile_image_url) VALUES ($1,$2,$3)",
                     &[&t.user.id, &t.user.name, &t.user.profile_image_url])
            .unwrap();
    }
    conn.execute("INSERT INTO tweet
            (id, user_id, text, in_reply_to_status_id, in_reply_to_user_id)
            VALUES ($1,$2,$3,$4,$5)",
                 &[&t.id, &t.user.id, &t.text, &t.in_reply_to_status_id, &t.in_reply_to_user_id])
        .unwrap();
    t
}

pub fn tweet(mut req: &mut Request) -> IronResult<Response> {
    let conn = get_pg_connection!(&req);
    let map = req.get_ref::<Params>().unwrap();
    let find_url = map.find(&["twitter_url"]);
    if find_url.is_none() {
        return Ok(Response::with(status::NotFound));
    }
    let url_value = find_url.unwrap();
    let re = regex::Regex::new(r"https://twitter.com/[^/]+/status/(\d+)").unwrap();
    if let &Value::String(ref url) = url_value {
        let raw_caps = re.captures(url);
        if raw_caps.is_none() {
            return Ok(Response::with(status::BadRequest));
        }
        let id = i64::from_str(raw_caps.unwrap()
                                       .get(1)
                                       .unwrap()
                                       .as_str())
                .unwrap();
        let mut tweets: Vec<Tweet> = Vec::new();
        let mut current_id = id;
        loop {
            let t = get_tweet(&conn, current_id);
            info!("{:?}", t);
            let next_id: Option<i64> = t.in_reply_to_status_id;
            tweets.insert(0, t);
            if next_id.is_none() {
                break;
            }
            current_id = next_id.unwrap();
        }
        info!("{:?}", tweets);
        return Ok(Response::with(((status::Found, RedirectRaw(format!("/tweet/{}", id))))));
    } else {
        unimplemented!();
    }
}

pub fn index(_: &mut Request) -> IronResult<Response> {
    let data = MapBuilder::new().build();
    Ok(Response::with((mime!(Text / Html),
                       status::Ok,
                       render_to_response("resources/templates/index.mustache", &data))))
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
    router.post("/tweet", tweet, "tweet");
    let mut chain = Chain::new(router);
    chain.link_before(logger_before);
    chain.link_after(logger_after);
    chain.link(PRead::<db::PostgresDB>::both(pool));
    info!("Gasconade booted");
    info!("Token is {:?}", *TOKEN);
    Iron::new(chain).http("0.0.0.0:8000").unwrap();
}
