// Copyright (c) 2018-2021 The MobileCoin Foundation

use assert_cmd::Command;
use maplit::btreeset;
use mc_api::external;
use mc_attest_net::{Client as AttestClient, RaClient};
use mc_common::logger::{test_with_logger, Logger};
use mc_fog_api::ingest_common::IngestSummary;
use mc_fog_ingest_server::server::{IngestServer, IngestServerConfig};
use mc_fog_sql_recovery_db::{test_utils::SqlRecoveryDbTestContext, SqlRecoveryDb};
use mc_fog_test_infra::get_enclave_path;
use mc_fog_uri::{ConnectionUri, FogIngestUri, IngestPeerUri};
use mc_ledger_db::LedgerDB;
use mc_watcher::watcher_db::WatcherDB;
use predicates::prelude::*;
use std::{str::FromStr, time::Duration};
use tempdir::TempDir;

const OMAP_CAPACITY: u64 = 256;
const BASE_PORT: u32 = 3220;

#[test_with_logger]
fn test_sync_keys_from_remote(logger: Logger) {
    let ingest_server_set_up_data = set_up_ingest_servers(logger);

    let mut cmd = Command::cargo_bin("fog_ingest_client").unwrap();
    cmd.arg("--uri")
        .arg(ingest_server_set_up_data.backup_ingest_server_client_uri)
        .arg("sync-keys-from-remote")
        .arg(ingest_server_set_up_data.primary_ingest_server_peer_uri)
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "{}",
            hex::encode(
                ingest_server_set_up_data
                    .primary_ingest_server_ingress_pubkey
                    .get_data()
            )
        )));
}

/// Contains data pertaining to the set up of the two ingest servers that are
/// used in the test.
struct IngestServerSetUpData {
    /// The primary ingest server that will provide the back up server with its
    /// ingress public key.
    _primary_ingest_server: IngestServer<AttestClient, SqlRecoveryDb>,
    /// The back up ingest server that will field grpc requests from our command
    /// line program and sync the ingress public key from the primary ingest
    /// server.
    _backup_ingest_server: IngestServer<AttestClient, SqlRecoveryDb>,
    /// The db context that is used to created the RecoveryDb. This must be
    /// included here to ensure that the RecoveryDb is not dropped at the end
    /// of the set up method.
    _db_test_context: SqlRecoveryDbTestContext,
    /// The url that can be used to address the backup ingest server of grpc.
    backup_ingest_server_client_uri: String,
    /// The url that can be used to address by a peer ingest server to
    /// addressthe primary ingest server.
    primary_ingest_server_peer_uri: String,
    /// The primary ingest server's ingress public key.
    primary_ingest_server_ingress_pubkey: external::CompressedRistretto,
}

fn set_up_ingest_servers(logger: Logger) -> IngestServerSetUpData {
    let db_test_context = SqlRecoveryDbTestContext::new(logger.clone());
    let db = db_test_context.get_db_instance();

    let primary_ingest_server_peer_uri_str = format!("insecure-igp://127.0.0.1:{}/", BASE_PORT + 5);
    let primary_node_ingest_summary: IngestSummary;

    // Set up the primary ingest server
    let (primary_ingest_server, primary_ingest_server_ingress_pubkey) = {
        let igp_uri = IngestPeerUri::from_str(&primary_ingest_server_peer_uri_str).unwrap();
        let local_node_id = igp_uri.responder_id().unwrap();

        let config = IngestServerConfig {
            ias_spid: Default::default(),
            local_node_id: local_node_id.clone(),
            client_listen_uri: FogIngestUri::from_str(&format!(
                "insecure-fog-ingest://0.0.0.0:{}/",
                BASE_PORT + 4
            ))
            .unwrap(),
            peer_listen_uri: igp_uri.clone(),
            peers: btreeset![igp_uri.clone()],
            fog_report_id: Default::default(),
            max_transactions: 10_000,
            pubkey_expiry_window: 100,
            peer_checkup_period: None,
            watcher_timeout: Duration::default(),
            state_file: None,
            enclave_path: get_enclave_path(mc_fog_ingest_enclave::ENCLAVE_FILE),
            omap_capacity: OMAP_CAPACITY,
        };

        // Set up the Watcher DB - create a new watcher DB for each phase
        let db_tmp = TempDir::new("wallet_db").expect("Could not make tempdir for wallet db");
        WatcherDB::create(db_tmp.path()).unwrap();
        let watcher = WatcherDB::open_ro(db_tmp.path(), logger.clone()).unwrap();

        // Set up an empty ledger db.
        let ledger_db_path =
            TempDir::new("ledger_db").expect("Could not make tempdir for ledger db");
        LedgerDB::create(ledger_db_path.path()).unwrap();
        let ledger_db = LedgerDB::open(ledger_db_path.path()).unwrap();

        let ra_client = AttestClient::new("").expect("Could not create IAS client");
        let mut node = IngestServer::new(
            config,
            ra_client,
            db.clone(),
            watcher,
            ledger_db,
            logger.clone(),
        );
        node.start().expect("Could not start Ingest Service");
        node.activate().expect("Could not activate Ingest");

        primary_node_ingest_summary = node.get_ingest_summary();
        let ingress_pubkey = primary_node_ingest_summary.get_ingress_pubkey();

        (node, ingress_pubkey)
    };

    std::thread::sleep(std::time::Duration::from_millis(1000));

    let backup_ingest_server_client_uri_str =
        format!("insecure-fog-ingest://0.0.0.0:{}/", BASE_PORT + 8);

    // Set up the backup ingest server
    let backup_ingest_server = {
        let igp_uri =
            IngestPeerUri::from_str(&format!("insecure-igp://127.0.0.1:{}/", BASE_PORT + 9))
                .unwrap();
        let local_node_id = igp_uri.responder_id().unwrap();

        let config = IngestServerConfig {
            ias_spid: Default::default(),
            local_node_id,
            client_listen_uri: FogIngestUri::from_str(&backup_ingest_server_client_uri_str)
                .unwrap(),
            peer_listen_uri: igp_uri.clone(),
            peers: btreeset![igp_uri.clone()],
            fog_report_id: Default::default(),
            max_transactions: 10_000,
            pubkey_expiry_window: 100,
            peer_checkup_period: Some(std::time::Duration::from_millis(10000)),
            watcher_timeout: Duration::default(),
            state_file: None,
            enclave_path: get_enclave_path(mc_fog_ingest_enclave::ENCLAVE_FILE),
            omap_capacity: OMAP_CAPACITY,
        };

        let db_tmp = TempDir::new("wallet_db").expect("Could not make tempdir for wallet db");
        WatcherDB::create(db_tmp.path()).unwrap();
        let watcher = WatcherDB::open_ro(db_tmp.path(), logger.clone()).unwrap();

        // Set up an empty ledger db.
        let ledger_db_path =
            TempDir::new("ledger_db").expect("Could not make tempdir for ledger db");
        LedgerDB::create(ledger_db_path.path()).unwrap();
        let ledger_db = LedgerDB::open(ledger_db_path.path()).unwrap();

        let ra_client = AttestClient::new("").expect("Could not create IAS client");
        let mut node = IngestServer::new(
            config,
            ra_client,
            db.clone(),
            watcher,
            ledger_db,
            logger.clone(),
        );

        node.start().expect("Could not start Ingest Service");
        node
    };

    return IngestServerSetUpData {
        _primary_ingest_server: primary_ingest_server,
        _backup_ingest_server: backup_ingest_server,
        _db_test_context: db_test_context,
        backup_ingest_server_client_uri: backup_ingest_server_client_uri_str.to_owned(),
        primary_ingest_server_peer_uri: primary_ingest_server_peer_uri_str.to_owned(),
        primary_ingest_server_ingress_pubkey: primary_ingest_server_ingress_pubkey.clone(),
    };
}