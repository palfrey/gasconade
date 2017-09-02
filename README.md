docker run -P -d postgres
DATABASE_URL=postgresql://postgres:postgres@localhost:32769 cargo watch -x run

https://twitter.com/PopSci/status/902216010505359361