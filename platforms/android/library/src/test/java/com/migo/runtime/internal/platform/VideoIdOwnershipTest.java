package com.migo.runtime.internal.platform;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.fail;

import org.json.JSONObject;
import org.junit.Test;

import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ConcurrentMap;

/**
 * The runtime owns the video id; Android honours it rather than inventing one.
 *
 * <p>Before this, {@code VideoManager} allocated from its own counter starting
 * at 1 while the runtime allocated from its own, and the two agreed only by
 * coincidence of both being reset at the same moments. They stop agreeing the
 * first time a runtime restarts, or the first time a create fails on one side
 * only — and then {@code onVideoEvent} carries an id that names a different
 * player in JavaScript than it does in Java.
 */
public final class VideoIdOwnershipTest {

    @Test
    public void the_id_the_runtime_assigned_is_the_id_that_is_used() throws Exception {
        assertEquals(77, VideoManager.requestedVideoId(new JSONObject("{\"videoId\":77}")));
        assertEquals(Integer.MAX_VALUE,
                VideoManager.requestedVideoId(new JSONObject("{\"videoId\":2147483647}")));
    }

    @Test
    public void a_create_without_a_usable_id_is_refused_rather_than_renumbered() throws Exception {
        // Each of these is what the runtime's own id space can never produce, so
        // accepting one would mean answering events under an id JavaScript does
        // not hold. `1.5` is the sharp one: a truncating parser reads it as 1,
        // and 1 is a real video.
        String[] rejected = {
            "{}",
            "{\"videoId\":0}",
            "{\"videoId\":-1}",
            "{\"videoId\":1.5}",
            "{\"videoId\":2147483648}",
            "{\"videoId\":true}",
            "{\"videoId\":null}",
            "{\"videoId\":\"7\"}",
        };
        for (String options : rejected) {
            try {
                int accepted = VideoManager.requestedVideoId(new JSONObject(options));
                fail(options + " was accepted as video id " + accepted);
            } catch (IllegalArgumentException refused) {
                // The message names the offending value so a host integrating
                // the SDK can tell this from a decode failure.
                assertEquals(options, true, refused.getMessage().contains("videoId"));
            }
        }
    }

    @Test
    public void an_id_already_live_is_refused_and_leaves_the_live_player_in_place() {
        ConcurrentMap<Integer, String> live = new ConcurrentHashMap<>();
        VideoManager.claimVideoId(live, 5, "first");

        try {
            VideoManager.claimVideoId(live, 5, "second");
            fail("a second claim on video id 5 was allowed");
        } catch (IllegalStateException refused) {
            assertEquals(true, refused.getMessage().contains("5"));
        }

        // The refusal must not have replaced the live player: the events of the
        // video that is actually playing would start going to a dead object.
        assertSame("first", live.get(5));
        assertEquals(1, live.size());
    }

    @Test
    public void distinct_ids_coexist() {
        ConcurrentMap<Integer, String> live = new ConcurrentHashMap<>();
        VideoManager.claimVideoId(live, 5, "first");
        VideoManager.claimVideoId(live, 6, "second");

        assertEquals(2, live.size());
        assertSame("first", live.get(5));
        assertSame("second", live.get(6));
    }
}
