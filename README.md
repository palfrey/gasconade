Gasconade
=========
[![dependency status](https://deps.rs/repo/github/palfrey/gasconade/status.svg)](https://deps.rs/repo/github/palfrey/gasconade)

_n. extravagant boasting; boastful talk_

In this context, it's a Rust web app for making blog posts for Twitter users who apparently think that 100-item threads are readable.

Local running
-------------

1. Have a Postgres database, and set environment variable DATABASE_URL to a login for that.
    * Easiest way is with Docker, using `docker run -P -d postgres` and then `export DATABASE_URL=postgresql://postgres:postgres@localhost:<port>` where you can get `port` from `docker ps`
2. [Install Rust](https://www.rust-lang.org/en-US/install.html)
3. Copy `config.yaml.example` to `config.yaml` and replace `key`/`secret` with keys from a [Twitter app you've registered](https://apps.twitter.com/)
4. `cargo run`
5. Open [http://localhost:8000](http://localhost:8000)

Heroku running
--------------

1. Get the [Heroku CLI](https://devcenter.heroku.com/articles/heroku-cli)
2. `heroku create --buildpack https://github.com/emk/heroku-buildpack-rust.git`
3. Add Heroku Postgres
4. Set TWITTER_KEY and TWITTER_SECRET config vars as per `key`/`secret` from local running