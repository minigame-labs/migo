# 这个 fixture 是为了回答哪个问题

MigoGLX（P3-G）的开始条件有两条：拿到真实 Unity/Emscripten 导出，**且**实测 JS glue
是热点。计划里写着「全仓一个 `.wasm` 都没有，今天连热点都测不出来」——这就是那个缺口。

`src/main.c` 是自己写的 Emscripten WebGL2 程序（不需要 Unity 授权、不用别人的游戏），
每帧发 2000 次小 draw、每次带两个 uniform 更新，刻意做成 glue 密集的形状。

## ★ 已有的 profiling 数据回答不了这个问题

`docs`/memory 里有一条现成结论：三类游戏的瓶颈类型学中 **WebGL 类是 JS/IC-bound**。
**不能拿它当「glue 是热点」的证据**——那批内容（Pixi / Phaser）是**手写 JS**，
根本没有 WASM→JS glue 这一层。它的热点在 JS 引擎自身。

MigoGLX 针对的是 **Emscripten 导出**：JS glue 是 WASM 和 WebGL 之间那层薄封装，
是一条独立的路径。所以这个测量必须单独做，旧数据既不能证实也不能证伪。

## 这个 fixture 能给出什么、不能给出什么

- **能**：WASM→JS→op 这条路上**每次 GL 调用的成本**。
- **不能**：「glue 是不是热点」。那取决于**真实内容的调用次数**，而这个 fixture 是
  刻意做密的，比值会偏向「glue 重要」。

正确用法是：拿这里的每调用成本 × 真实内容的每帧调用数，再和帧预算比。
现有的 bunnymark / endless-runner 已经能给出后者的量级。

## 构建

```bash
source ~/emsdk/emsdk_env.sh
emcc src/main.c -o game.js -sMAX_WEBGL_VERSION=2 -sMIN_WEBGL_VERSION=2 \
     -sALLOW_MEMORY_GROWTH=1 -O2
```
