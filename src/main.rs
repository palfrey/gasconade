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
    for row in &conn.query("SELECT text, in_reply_to_status_id, in_reply_to_user_id FROM tweet",
                           &[])
                    .unwrap() {
        return Tweet {
                   id: id,
                   user: TwitterUser {
                       id: -1,
                       name: String::new(),
                       profile_image_url: String::new(),
                   },
                   text: row.get(0),
                   in_reply_to_status_id: row.get(1),
                   in_reply_to_user_id: row.get(2),
               };
    }
    let client = reqwest::Client::new().unwrap();
    let mut res = client.get(&format!("https://api.twitter.com/1.1/statuses/show.json?id={}", id))
        .unwrap()
        .header(Authorization(Bearer { token: TOKEN.clone() }))
        .send()
        .unwrap();
    return res.json().unwrap();
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
        let t = get_tweet(&conn, id);
        println!("{:?}", t);
        return Ok(Response::with((status::Ok, format!("{}", id))));
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
