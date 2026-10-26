# -*- coding: utf-8 -*-
# Client stub for the HarnessControl gRPC service (harness -> control plane).
#
# Hand-written to mirror `grpc_tools` output for `proto/control.proto` (the harness
# only needs the client side). Regenerate with grpcio-tools if the proto changes:
#   python -m grpc_tools.protoc -I proto --python_out=... --grpc_python_out=... proto/control.proto

import grpc

import control_pb2 as control__pb2


class HarnessControlStub(object):
    """The harness opens one bidirectional stream per run (PLAN.md §6/§9)."""

    def __init__(self, channel):
        self.Connect = channel.stream_stream(
            '/cat_herder.v1.HarnessControl/Connect',
            request_serializer=control__pb2.HarnessMessage.SerializeToString,
            response_deserializer=control__pb2.ControlMessage.FromString,
        )


class HarnessControlServicer(object):
    """Server side (not used by the harness — the Rust control plane hosts it)."""

    def Connect(self, request_iterator, context):
        context.set_code(grpc.StatusCode.UNIMPLEMENTED)
        context.set_details('Method not implemented!')
        raise NotImplementedError('Method not implemented!')


def add_HarnessControlServicer_to_server(servicer, server):
    rpc_method_handlers = {
        'Connect': grpc.stream_stream_rpc_method_handler(
            HarnessControlServicer.Connect,
            request_deserializer=control__pb2.HarnessMessage.FromString,
            response_serializer=control__pb2.ControlMessage.SerializeToString,
        ),
    }
    generic_handler = grpc.method_handlers_generic_handler(
        'cat_herder.v1.HarnessControl', rpc_method_handlers)
    server.add_generic_rpc_handlers((generic_handler,))
