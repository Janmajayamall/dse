use async_trait::async_trait;
use futures::{AsyncRead, AsyncWrite};
use libp2p::core::upgrade::{read_length_prefixed, write_length_prefixed, SelectUpgrade};
use libp2p::core::ProtocolName;
use libp2p::request_response::{RequestResponseCodec};
use serde::{de::DeserializeOwned, ser::Serialize};
use std::io;
use std::marker::PhantomData;

#[derive(Clone)]
pub struct RequestResponse<P, REQ, RES> {
    protocol: PhantomData<P>,
    request: PhantomData<REQ>,
    response: PhantomData<RES>,
}

impl<P, REQ, RES> Default for RequestResponse<P, REQ, RES> {
    fn default() -> Self {
        Self {
            protocol: PhantomData::<P>::default(),
            request: PhantomData::<REQ>::default(),
            response: PhantomData::<RES>::default(),
        }
    }
}

#[async_trait]
impl<P, REQ, RES> RequestResponseCodec for RequestResponse<P, REQ, RES>
where
    P: ProtocolName + Sync + Send + Clone,
    REQ: Sized + Serialize + DeserializeOwned + Clone + Sync + Send,
    RES: Sized + Serialize + DeserializeOwned + Clone + Sync + Send,
{
    type Protocol = P;
    type Request = REQ;
    type Response = RES;

    async fn read_request<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let vec = read_length_prefixed(io, 1000000).await?;
        match serde_json::from_slice(&vec) {
            Ok(v) => Ok(v),
            Err(e) => Err(io::ErrorKind::Other.into()),
        }
    }

    async fn read_response<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        let vec = read_length_prefixed(io, 1000000).await?;
        match serde_json::from_slice(&vec) {
            Ok(v) => Ok(v),
            Err(_) => Err(io::ErrorKind::Other.into()),
        }
    }

    async fn write_request<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
        req: Self::Request,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        match serde_json::to_vec(&req) {
            Ok(val) => {
                write_length_prefixed(io, &val).await?;
                Ok(())
            }
            Err(_) => Err(io::ErrorKind::Other.into()),
        }
    }

    async fn write_response<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
        res: Self::Response,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        match serde_json::to_vec(&res) {
            Ok(val) => {
                write_length_prefixed(io, &val).await?;
                Ok(())
            }
            Err(_) => Err(io::ErrorKind::Other.into()),
        }
    }
}
