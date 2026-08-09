package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.json.JSONObject;
import org.junit.Test;

/**
 * The correlation id a deferred result carries back.
 *
 * <p>The runtime matches a reply to its request by this id and, when the key is
 * missing entirely, falls back to settling the oldest pending request. Both
 * halves matter here: a reply that carries the right id must correlate, and a
 * reply to a request that had no id must leave the key off rather than invent
 * one, because a value the runtime rejects loses the fallback as well.
 */
public class CallbackCorrelationTest {

    @Test
    public void an_id_is_read_from_the_options_a_request_carried() throws Exception {
        assertEquals(77, CallbackCorrelation.requestIdOf("{\"requestId\":77,\"type\":\"gcj02\"}"));
        assertEquals(77, CallbackCorrelation.requestIdOf(new JSONObject().put("requestId", 77)));
    }

    @Test
    public void anything_that_is_not_a_positive_int_reads_as_absent() throws Exception {
        // Every one of these is what the runtime's own parser rejects. Reading
        // any of them as an id would let a reply claim a request it does not
        // answer -- `true` is the sharp one, because a coercing parser turns it
        // into the id 1 and 1 is a real id.
        String[] rejected = {
            "{}",
            "{\"requestId\":0}",
            "{\"requestId\":-1}",
            "{\"requestId\":1.5}",
            // Arrives as a Long and is one past the signed 32-bit space the
            // id crosses JNI in, so it is not an id however it is spelled.
            "{\"requestId\":2147483648}",
            "{\"requestId\":true}",
            "{\"requestId\":null}",
            "{\"requestId\":\"abc\"}",
            "{\"requestId\":[]}",
            "{\"requestId\":{}}",
            "not json at all",
            "",
        };
        for (String options : rejected) {
            assertEquals(options, CallbackCorrelation.ABSENT, CallbackCorrelation.requestIdOf(options));
        }
        assertEquals(CallbackCorrelation.ABSENT, CallbackCorrelation.requestIdOf((String) null));
        assertEquals(CallbackCorrelation.ABSENT, CallbackCorrelation.requestIdOf((JSONObject) null));
    }

    @Test
    public void a_failure_answers_the_request_that_asked() throws Exception {
        JSONObject failure = new JSONObject(
                CallbackCorrelation.failure(42, "getLocation", "location service disabled"));

        assertEquals(42, failure.getInt("requestId"));
        assertEquals("getLocation:fail location service disabled", failure.getString("error"));
    }

    @Test
    public void a_failure_to_an_id_less_request_carries_no_id_at_all() throws Exception {
        JSONObject failure = new JSONObject(
                CallbackCorrelation.failure(CallbackCorrelation.ABSENT, "getLocation", "no activity"));

        // Not "requestId: 0". The runtime rejects a non-positive id as *present
        // and invalid* and discards the reply, where an omitted key still
        // settles through the compatibility fallback.
        assertFalse(failure.has("requestId"));
        assertEquals("getLocation:fail no activity", failure.getString("error"));
    }

    @Test
    public void a_failure_reason_with_quotes_stays_one_json_document() throws Exception {
        JSONObject failure = new JSONObject(
                CallbackCorrelation.failure(7, "scanCode", "camera \"busy\" \\ elsewhere"));

        assertEquals(7, failure.getInt("requestId"));
        assertEquals("scanCode:fail camera \"busy\" \\ elsewhere", failure.getString("error"));
    }

    @Test
    public void a_null_reason_still_produces_a_readable_failure() throws Exception {
        JSONObject failure = new JSONObject(CallbackCorrelation.failure(3, "getLocation", null));

        assertEquals(3, failure.getInt("requestId"));
        assertEquals("getLocation:fail unknown error", failure.getString("error"));
    }

    @Test
    public void stamping_is_the_same_decision_as_reading() throws Exception {
        JSONObject present = new JSONObject();
        CallbackCorrelation.stamp(present, 9);
        assertTrue(present.has("requestId"));
        assertEquals(9, present.getInt("requestId"));

        JSONObject absent = new JSONObject();
        CallbackCorrelation.stamp(absent, CallbackCorrelation.ABSENT);
        assertFalse(absent.has("requestId"));

        // A negative id cannot arrive from the allocator, but it can arrive from
        // a caller that computed one; it must not be stamped either.
        JSONObject negative = new JSONObject();
        CallbackCorrelation.stamp(negative, -5);
        assertFalse(negative.has("requestId"));
    }

    @Test
    public void an_id_round_trips_from_request_options_to_its_result() throws Exception {
        // The whole property in one place: what a manager reads out of the
        // request is what the runtime finds in the reply.
        String options = "{\"requestId\":123456,\"type\":\"wgs84\"}";
        JSONObject result = new JSONObject().put("latitude", 1.0).put("longitude", 2.0);

        CallbackCorrelation.stamp(result, CallbackCorrelation.requestIdOf(options));

        assertEquals(123456, new JSONObject(result.toString()).getInt("requestId"));
    }
}
