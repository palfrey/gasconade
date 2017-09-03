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
extern crate rustc_serialize;

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
use db::PostgresConnection;

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

#[derive(Deserialize, Debug, RustcEncodable)]
struct TwitterUser {
    id: i64,
    screen_name: String,
    name: String,
    profile_image_url: String,
}

#[derive(Deserialize, Debug, RustcEncodable)]
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
struct OEmbed {
    html: String,
}

fn get_tweet(conn: &db::PostgresConnection, id: i64) -> Tweet {
    let tweets = &conn.query("
    SELECT user_id, text,
    in_reply_to_status_id,
    in_reply_to_user_id, html FROM tweet WHERE id = $1",
                             &[&id])
                      .unwrap();
    if !tweets.is_empty() {
        let tweet = tweets.get(0);
        let user_id: i64 = tweet.get(0);
        let users = &conn.query("
            SELECT name, profile_image_url, username FROM twitter_user WHERE id = $1",
                                &[&user_id])
                         .unwrap();
        let user = users.get(0);
        return Tweet {
                   id: id,
                   user: TwitterUser {
                       id: user_id,
                       name: user.get(0),
                       profile_image_url: user.get(1),
                       screen_name: user.get(2),
                   },
                   text: tweet.get(1),
                   in_reply_to_status_id: tweet.get(2),
                   in_reply_to_user_id: tweet.get(3),
                   html: tweet.get(4),
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
        conn.execute("
        INSERT INTO twitter_user (id, name, profile_image_url, username) VALUES ($1,$2,$3,$4)",
                     &[&t.user.id, &t.user.name, &t.user.profile_image_url, &t.user.screen_name])
            .unwrap();
    }
    let mut content = client.get(&format!("https://publish.twitter.com/oembed?
url=https://twitter.com/{}/status/{}&hide_thread=true&omit_script=true&dnt=true",
                                          &t.user.screen_name,
                                          &t.id))
        .unwrap()
        .send()
        .unwrap();
    let oembed: OEmbed = content.json().expect("valid oembed data");
    conn.execute("INSERT INTO tweet
            (id, user_id, text, in_reply_to_status_id, in_reply_to_user_id, html)
            VALUES ($1,$2,$3,$4,$5,$6)",
                 &[&t.id,
                   &t.user.id,
                   &t.text,
                   &t.in_reply_to_status_id,
                   &t.in_reply_to_user_id,
                   &oembed.html])
        .unwrap();
    t
}

fn get_tweets(conn: &PostgresConnection, id: i64) -> Vec<Tweet> {
    let mut tweets: Vec<Tweet> = Vec::new();
    let mut current_id = id;
    loop {
        let t = get_tweet(&conn, current_id);
        let next_id: Option<i64> = t.in_reply_to_status_id;
        tweets.insert(0, t);
        if next_id.is_none() {
            break;
        }
        current_id = next_id.unwrap();
    }
    tweets
}

pub fn new_tweet(mut req: &mut Request) -> IronResult<Response> {
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
        get_tweets(&conn, id);
        return Ok(Response::with(((status::Found, RedirectRaw(format!("/tweet/{}", id))))));
    } else {
        unimplemented!();
    }
}

pub fn tweet(req: &mut Request) -> IronResult<Response> {
    let conn = get_pg_connection!(&req);
    let tweet_id: i64 = i64::from_str(req.extensions
                                          .get::<Router>()
                                          .unwrap()
                                          .find("tweet_id")
                                          .unwrap())
            .unwrap();
    let tweets = get_tweets(&conn, tweet_id);
    let data = MapBuilder::new()
        .insert("tweets", &tweets).expect("inserting tweets works")
        .insert_str("title", format!("Gasconade - {}", tweets[0].user.name))
        .build();
    Ok(Response::with((mime!(Text / Html),
                       status::Ok,
                       render_to_response("resources/templates/tweet.mustache", &data))))
}

pub fn index(_: &mut Request) -> IronResult<Response> {
    let data = MapBuilder::new()
        .insert_str("title", "Gasconade")
        .build();
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
    router.post("/tweet", new_tweet, "query tweet");
    router.get("/tweet/:tweet_id", tweet, "tweet");
    let mut chain = Chain::new(router);
    chain.link_before(logger_before);
    chain.link_after(logger_after);
    chain.link(PRead::<db::PostgresDB>::both(pool));
    info!("Gasconade booted");
    info!("Token is {:?}", *TOKEN);
    Iron::new(chain).http("0.0.0.0:8000").unwrap();
}
