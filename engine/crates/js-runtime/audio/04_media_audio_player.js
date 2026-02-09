import {
  op_media_audio_player_create,
  op_media_audio_player_add_source,
  op_media_audio_player_remove_source,
  op_media_audio_player_start,
  op_media_audio_player_stop,
  op_media_audio_player_destroy,
} from "ext:core/ops";

let nextMediaPlayerId = 1;

class MediaAudioPlayer {
  #id;
  #destroyed = false;

  constructor() {
    this.#id = nextMediaPlayerId++;
    op_media_audio_player_create(this.#id);
  }

  addAudioSource(source) {
    if (this.#destroyed) return this;
    if (source && typeof source._getId === "function") {
      op_media_audio_player_add_source(this.#id, source._getId());
    }
    return this;
  }

  removeAudioSource(source) {
    if (this.#destroyed) return this;
    if (source && typeof source._getId === "function") {
      op_media_audio_player_remove_source(this.#id, source._getId());
    }
    return this;
  }

  start() {
    if (this.#destroyed) return;
    op_media_audio_player_start(this.#id);
  }

  stop() {
    if (this.#destroyed) return;
    op_media_audio_player_stop(this.#id);
  }

  destroy() {
    if (this.#destroyed) return;
    this.#destroyed = true;
    op_media_audio_player_destroy(this.#id);
  }
}

function createMediaAudioPlayer() {
  return new MediaAudioPlayer();
}

export { MediaAudioPlayer, createMediaAudioPlayer };
