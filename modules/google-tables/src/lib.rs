pub mod columns;
pub mod cycle;
pub mod google;
pub mod limiter;
pub mod model;
pub mod scan;
pub mod server;
pub mod session;
pub mod sheets;
pub mod store;
pub mod uid;
pub mod values;
pub mod watch;

pub mod irbis {
    pub mod table {
        pub mod v1 {
            tonic::include_proto!("irbis.table.v1");
        }
    }
    pub mod google_tables {
        pub mod v1 {
            tonic::include_proto!("irbis.google_tables.v1");
        }
    }
}
