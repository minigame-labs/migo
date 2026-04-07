package com.migo.runtime;

/**
 * Lifecycle states of a {@link GameSession}.
 *
 * <pre>
 * CREATED ──► RUNNING ──► PAUSED ──► RUNNING (resume)
 *    │            │           │
 *    └────────────┴───────────┴──► DESTROYED
 * </pre>
 */
public enum SessionState {
    /** Session created, waiting for {@link GameSession#startGame} call. */
    CREATED,
    /** Game code loaded and actively running. */
    RUNNING,
    /** Activity paused; rendering suspended, audio muted. */
    PAUSED,
    /** Terminal state. Session released, no further operations allowed. */
    DESTROYED
}
