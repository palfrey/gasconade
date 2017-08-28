use postgres;
use postgres::transaction::Transaction;
use schemamama;
use schemamama::Migrator;
use schemamama_postgres::{PostgresAdapter, PostgresMigration};

// struct Nodes;
// migration!(Nodes, 201610221748, "add other node listing");

// impl PostgresMigration for Nodes {
//     fn up(&self, transaction: &Transaction) -> Result<(), postgres::error::Error> {
//         transaction.execute("CREATE TABLE nodes (url VARCHAR(2083) PRIMARY KEY);", &[])
//             .unwrap();
//         return Ok(());
//     }

//     fn down(&self, transaction: &Transaction) -> Result<(), postgres::error::Error> {
//         let _ = transaction.execute("DROP TABLE nodes", &[]).unwrap();
//         return Ok(());
//     }
// }

fn migrate(connection: &postgres::Connection) -> Migrator<PostgresAdapter> {
    let adapter = PostgresAdapter::new(connection);
    let _ = adapter.setup_schema().unwrap();

    let mut migrator = Migrator::new(adapter);
    //migrator.register(Box::new(Nodes));
    return migrator;
}

pub fn up(connection: &postgres::Connection) -> Result<(), schemamama::Error<postgres::error::Error>> {
    let migrator = migrate(connection);
    return migrator.up(None);
}