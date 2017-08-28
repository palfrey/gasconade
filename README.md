docker run -P -d postgres
DATABASE_URL=postgresql://postgres:postgres@localhost:32768 cargo watch -x check -x build -x run