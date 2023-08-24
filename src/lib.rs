cargo_component_bindings::generate!();

use bindings::exports::wasi::http::incoming_handler::IncomingHandler;
use bindings::wasi::http::types::{IncomingRequest, ResponseOutparam};

use crate::bindings::wasi::http::outgoing_handler;
use crate::bindings::wasi::http::types::{
    future_incoming_response_get, incoming_request_authority, incoming_request_consume,
    incoming_request_headers, incoming_request_method, incoming_request_path_with_query,
    incoming_request_scheme, incoming_response_consume, incoming_response_headers,
    incoming_response_status, listen_to_future_incoming_response, new_outgoing_request,
    new_outgoing_response, outgoing_request_write, outgoing_response_write, set_response_outparam,
};
use crate::bindings::wasi::io::streams::forward;
use crate::bindings::wasi::poll::poll::poll_oneoff;

struct Component;

impl IncomingHandler for Component {
    /// A passthrough HTTP proxy.
    ///
    /// Getting my feet wet with an HTTP proxy that simply forwards the request it receives
    /// unchanged to the `outgoing-handler` it imports, and then forwards the response from that
    /// handler back to its caller.
    ///
    /// Things I'm unsure about:
    ///
    /// - Do I need to be calling the `drop_*` functions on these various types, or is that done
    ///   implicitly by providing such a resource as an argument to, e.g.,
    ///   `outgoing_handler::handle`? What are the consequences for failing to drop at the right
    ///   time?
    ///
    /// - What operations are valid to perform on the incoming/outgoing streams, and when? In my
    ///   draft here, I call `forward` before invoking `outgoing_handler::handle` and
    ///   `set_response_outparam`. Is that okay?
    ///
    /// - Am I using `poll_oneoff` correctly?
    fn handle(req: IncomingRequest, resp_out: ResponseOutparam) {
        let req_body = incoming_request_consume(req).expect("can get the req stream for reading");

        // Move the request over from an `incoming-request` to an `outgoing-request`
        let bereq = new_outgoing_request(
            &incoming_request_method(req),
            incoming_request_path_with_query(req).as_deref(),
            incoming_request_scheme(req).as_ref(),
            incoming_request_authority(req).as_deref(),
            incoming_request_headers(req),
        );
        let bereq_body =
            outgoing_request_write(bereq).expect("can get the bereq stream for writing");

        // Is this too early? Do we need to call `outgoing_handler::handle()` first to get things
        // off the ground?
        forward(bereq_body, req_body)
            .expect("can pipe the body through from client request to backend request");

        // Send the request to the backend by invoking our imported `outgoing_handler`
        let beresp_fut = outgoing_handler::handle(bereq, None);

        // Do the dance to register interest in the future, poll that interest, and assert that we
        // did in fact get a response
        let beresp_fut_poll = listen_to_future_incoming_response(beresp_fut);
        let poll_result = poll_oneoff(&[beresp_fut_poll]);
        assert!(poll_result[0]);

        // Actually get our hands on the backend response and its body
        let beresp = future_incoming_response_get(beresp_fut)
            .expect("the future is ready")
            .expect("and the request succeeded");
        let beresp_body =
            incoming_response_consume(beresp).expect("can get the beresp stream for reading");

        // Move the response over from an `incoming-response` to an `outgoing-response`
        let resp = new_outgoing_response(
            incoming_response_status(beresp),
            incoming_response_headers(beresp),
        );
        let resp_body = outgoing_response_write(resp).expect("can get the resp stream for writing");

        // Again, does it matter whether this is called before or after getting the response started?
        forward(resp_body, beresp_body)
            .expect("can pipe the body through from the backend response to the client response");

        // Finally, we set the client response and are done
        set_response_outparam(resp_out, Ok(resp)).expect("can set the outgoing resp");
    }
}
