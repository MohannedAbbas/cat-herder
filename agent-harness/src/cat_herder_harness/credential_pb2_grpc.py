# -*- coding: utf-8 -*-
# Client stub for the CredentialBroker gRPC service (harness sidecar -> broker).
#
# Hand-written to mirror `grpc_tools` output for `proto/credential.proto` (the harness
# only needs the client side). Regenerate with grpcio-tools if the proto changes:
#   python -m grpc_tools.protoc -I proto --python_out=... --grpc_python_out=... proto/credential.proto

import grpc

from . import credential_pb2 as credential__pb2


class CredentialBrokerStub(object):
    """Unary RPCs: mint/revoke/revoke_run (PLAN.md §11.3)."""

    def __init__(self, channel):
        self.Mint = channel.unary_unary(
            '/cat_herder.v1.CredentialBroker/Mint',
            request_serializer=credential__pb2.MintRequest.SerializeToString,
            response_deserializer=credential__pb2.MintResponse.FromString,
        )
        self.Revoke = channel.unary_unary(
            '/cat_herder.v1.CredentialBroker/Revoke',
            request_serializer=credential__pb2.RevokeRequest.SerializeToString,
            response_deserializer=credential__pb2.RevokeResponse.FromString,
        )
        self.RevokeRun = channel.unary_unary(
            '/cat_herder.v1.CredentialBroker/RevokeRun',
            request_serializer=credential__pb2.RevokeRunRequest.SerializeToString,
            response_deserializer=credential__pb2.RevokeRunResponse.FromString,
        )


class CredentialBrokerServicer(object):
    """Server side (not used by the harness — the Rust broker hosts it)."""

    def Mint(self, request, context):
        context.set_code(grpc.StatusCode.UNIMPLEMENTED)
        context.set_details('Method not implemented!')
        raise NotImplementedError('Method not implemented!')

    def Revoke(self, request, context):
        context.set_code(grpc.StatusCode.UNIMPLEMENTED)
        context.set_details('Method not implemented!')
        raise NotImplementedError('Method not implemented!')

    def RevokeRun(self, request, context):
        context.set_code(grpc.StatusCode.UNIMPLEMENTED)
        context.set_details('Method not implemented!')
        raise NotImplementedError('Method not implemented!')


def add_CredentialBrokerServicer_to_server(servicer, server):
    rpc_method_handlers = {
        'Mint': grpc.unary_unary_rpc_method_handler(
            CredentialBrokerServicer.Mint,
            request_deserializer=credential__pb2.MintRequest.FromString,
            response_serializer=credential__pb2.MintResponse.SerializeToString,
        ),
        'Revoke': grpc.unary_unary_rpc_method_handler(
            CredentialBrokerServicer.Revoke,
            request_deserializer=credential__pb2.RevokeRequest.FromString,
            response_serializer=credential__pb2.RevokeResponse.SerializeToString,
        ),
        'RevokeRun': grpc.unary_unary_rpc_method_handler(
            CredentialBrokerServicer.RevokeRun,
            request_deserializer=credential__pb2.RevokeRunRequest.FromString,
            response_serializer=credential__pb2.RevokeRunResponse.SerializeToString,
        ),
    }
    generic_handler = grpc.method_handlers_generic_handler(
        'cat_herder.v1.CredentialBroker', rpc_method_handlers)
    server.add_generic_rpc_handlers((generic_handler,))
