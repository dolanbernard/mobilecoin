// Copyright (c) 2018-2022 The MobileCoin Foundation

//! A gRPC API module for attestation

mod autogenerated_code {
    // Expose proto data types from included third-party/external proto files.
    pub use protobuf::well_known_types::Empty;

    // Needed due to how to the auto-generated code references the Empty message.
    pub mod empty {
        pub use protobuf::well_known_types::Empty;
    }

    // Include the auto-generated code.
    include!(concat!(env!("OUT_DIR"), "/protos-auto-gen/mod.rs"));
}

pub mod conversions;
pub use autogenerated_code::*;
