// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::callback::{self, Callback};
use bytes::BytesMut;
use core::{
    marker::PhantomData,
    task::{Context, Poll},
};
use s2n_quic_core::{
    crypto::{tls, CryptoError, CryptoSuite},
    endpoint, transport,
};
use s2n_quic_ring::RingCryptoSuite;
use s2n_tls::raw::{
    config::{Config, ConfigResolver},
    connection::Connection,
    error::Error,
    ffi::{s2n_blinding, s2n_mode},
};

#[derive(Debug)]
pub struct Session {
    endpoint: endpoint::Type,
    pub(crate) connection: Connection,
    state: callback::State,
    handshake_complete: bool,
    send_buffer: BytesMut,
    config_resolver: Option<Box<dyn ConfigResolver>>,
}

impl Session {
    pub fn new(
        endpoint: endpoint::Type,
        config: Config,
        params: &[u8],
        config_resolver: Option<Box<dyn ConfigResolver>>,
    ) -> Result<Self, Error> {
        let mut connection = Connection::new(match endpoint {
            endpoint::Type::Server => s2n_mode::SERVER,
            endpoint::Type::Client => s2n_mode::CLIENT,
        });

        connection.set_config(config)?;
        connection.enable_quic()?;
        connection.set_quic_transport_parameters(params)?;
        // QUIC handles sending alerts, so no need to apply TLS blinding
        connection.set_blinding(s2n_blinding::SELF_SERVICE_BLINDING)?;

        Ok(Self {
            endpoint,
            connection,
            state: Default::default(),
            handshake_complete: false,
            send_buffer: BytesMut::new(),
            config_resolver,
        })
    }
}

impl CryptoSuite for Session {
    type HandshakeKey = <RingCryptoSuite as CryptoSuite>::HandshakeKey;
    type HandshakeHeaderKey = <RingCryptoSuite as CryptoSuite>::HandshakeHeaderKey;
    type InitialKey = <RingCryptoSuite as CryptoSuite>::InitialKey;
    type InitialHeaderKey = <RingCryptoSuite as CryptoSuite>::InitialHeaderKey;
    type OneRttKey = <RingCryptoSuite as CryptoSuite>::OneRttKey;
    type OneRttHeaderKey = <RingCryptoSuite as CryptoSuite>::OneRttHeaderKey;
    type ZeroRttKey = <RingCryptoSuite as CryptoSuite>::ZeroRttKey;
    type ZeroRttHeaderKey = <RingCryptoSuite as CryptoSuite>::ZeroRttHeaderKey;
    type RetryKey = <RingCryptoSuite as CryptoSuite>::RetryKey;
}

impl tls::Session for Session {
    fn poll<W>(&mut self, context: &mut W) -> Poll<Result<(), transport::Error>>
    where
        W: tls::Context<Self>,
    {
        let mut callback: Callback<W, Self> = Callback {
            context,
            endpoint: self.endpoint,
            state: &mut self.state,
            suite: PhantomData,
            err: None,
            send_buffer: &mut self.send_buffer,
        };

        unsafe {
            // let mut ctx = Context::from_waker(context.waker());

            // match &self.config_resolver {
            //     Some(config_resolver) => {
            //         let client_hello = (true, true);
            //         match config_resolver.poll_config(&mut ctx, client_hello) {
            //             Poll::Ready(Ok(config)) => {
            //                 // self.config.set_client_hello_callback();
            //                 //         self.config.set_client_hello_callback(callback, context);
            //                 //         self.config
            //                 //             .set_client_hello_callback_mode(s2n_client_hello_cb_mode::NONBLOCKING)?;
            //             }
            //             Poll::Ready(Err(err)) => {
            //                 return Poll::Ready(Err(transport::Error::NO_ERROR))
            //             }
            //             Poll::Pending => return Poll::Pending,
            //         }
            //     }
            //     None => (),
            // }

            // Safety: the callback struct must live as long as the callbacks are
            // set on on the connection
            callback.set(&mut self.connection);
        }

        let result = self.connection.negotiate().map_ok(|_| ());

        callback.unset(&mut self.connection)?;

        match result {
            Poll::Ready(Ok(())) => {
                // only emit handshake done once
                if !self.handshake_complete {
                    context.on_handshake_complete()?;
                    self.handshake_complete = true;
                }
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e
                .alert()
                .map(CryptoError::new)
                .unwrap_or(CryptoError::HANDSHAKE_FAILURE)
                .into())),
            Poll::Pending => Poll::Pending,
        }
    }
}
