use postgres::transaction::Transaction;
use schemamama::Migrator;
use schemamama_postgres::{PostgresAdapter, PostgresMigration};

struct Tweet;
migration!(Tweet, 201709021820, "add basic tweet listing");

impl PostgresMigration for Tweet {
    fn up(&self, transaction: &Transaction) -> Result<(), postgres::error::Error> {
        transaction
            .execute(
                "CREATE TABLE twitter_user (
                    id bigint PRIMARY KEY,
                    username VARCHAR(128) NOT NULL,
                    name VARCHAR(300) NOT NULL,
                    profile_image_url TEXT
                );",
                &[],
            )
            .unwrap();
        transaction
            .execute(
                "CREATE TABLE tweet (
                    id bigint PRIMARY KEY,
                    user_id bigint references twitter_user(id),
                    text VARCHAR(300) NOT NULL,
                    in_reply_to_status_id bigint NULL,
                    in_reply_to_user_id bigint NULL,
                    html TEXT
                );",
                &[],
            )
            .unwrap();
        Ok(())
    }

    fn down(&self, transaction: &Transaction) -> Result<(), postgres::error::Error> {
        transaction.execute("DROP TABLE tweet", &[]).unwrap();
        transaction.execute("DROP TABLE twitter_user", &[]).unwrap();
        Ok(())
    }
}

struct EmbedContent;
migration!(EmbedContent, 202212172302, "add full content data");

impl PostgresMigration for EmbedContent {
    fn up(&self, transaction: &Transaction) -> Result<(), postgres::error::Error> {
        transaction
            .execute(
                "ALTER TABLE tweet
                RENAME COLUMN html TO content;",
                &[],
            )
            .unwrap();
            transaction
            .execute(
                "ALTER TABLE tweet
                ADD COLUMN header TEXT DEFAULT '',
                ADD COLUMN footer TEXT DEFAULT '';",
                &[],
            )
            .unwrap();            
        Ok(())
    }

    fn down(&self, transaction: &Transaction) -> Result<(), postgres::error::Error> {
        transaction
            .execute(
                "ALTER TABLE tweet
                RENAME COLUMN content TO html,
                DROP COLUMN header RESTRICT,
                DROP COLUMN footer RESTRICT;",
                &[],
            )
            .unwrap();
        Ok(())
    }
}

fn migrate(connection: &postgres::Connection) -> Migrator<PostgresAdapter> {
    let adapter = PostgresAdapter::new(connection);
    adapter.setup_schema().unwrap();

    let mut migrator = Migrator::new(adapter);
    migrator.register(Box::new(Tweet));
    migrator.register(Box::new(EmbedContent));
    migrator
}

pub fn up(connection: &postgres::Connection) -> Result<(), schemamama::Error<postgres::error::Error>> {
    let migrator = migrate(connection);
    migrator.up(None)
}
