/*
 * The page exists to own an XComponent and nothing else.
 *
 * `libraryname` is what makes ArkUI load libmigohost.so and hand its native
 * module the XComponent instance; the native side then registers the surface
 * callbacks. Without it the component renders an empty surface and the native
 * code never learns a window exists -- which looks exactly like a rendering bug
 * rather than a wiring one.
 */

import migohost from 'libmigohost.so';

const CONTENT_PATH: string = '/data/storage/el2/base/haps/entry/files/content';

@Entry
@Component
struct Index {
  @State status: string = 'starting';

  build() {
    Stack() {
      XComponent({
        id: 'migo_surface',
        type: XComponentType.SURFACE,
        libraryname: 'migohost'
      })
        .onLoad(() => {
          // Started here rather than in aboutToAppear: onLoad fires once the
          // native module is loaded and bound to this component, so the native
          // side is guaranteed to exist before it is called into.
          const rc: number = migohost.start(CONTENT_PATH);
          this.status = rc === 0 ? 'engine started' : `start failed: ${rc}`;
          console.info(`[migo] start -> ${rc}`);
        })
        .width('100%')
        .height('100%')

      Text(this.status)
        .fontSize(14)
        .fontColor(Color.White)
        .backgroundColor('#80000000')
        .padding(6)
        .position({ x: 8, y: 8 })
    }
    .width('100%')
    .height('100%')
  }
}
