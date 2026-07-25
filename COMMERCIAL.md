# 商业许可 / Commercial Licensing

[English](#english) · 中文

Migo 以 [Business Source License 1.1](LICENSE)（BSL 1.1）发布。**绝大多数使用者不需要商业许可** —— 本页说明谁需要、谁不需要，以及怎么谈。

---

## 你**不需要**商业许可

以下用途由 BSL 1.1 直接授予，免费、无需联系我们、无需签任何东西：

- **阅读、审计、构建、测试、评测、修改、移植源码** —— 任何规模、任何主体都可以。BSL 的 Terms 把非生产用途无条件授予所有人，规模阈值**不适用于**它。
- **把 Migo 嵌进你自己的 App 上线**，只要你是 LICENSE 中定义的 **Small Entity**：
  - 主体及其关联方最近一个完整财年的**全部**年营业收入 ≤ **USD 1,000,000**；**且**
  - 该财年内任一月份，你的 App 全部实例的**月活跃用户 ≤ 3,000,000**。
- **开发、发行、销售跑在 Migo 上的小游戏** —— 游戏本身的收入不受影响，也不因此需要许可。
- 学术研究、教学、个人项目、开源项目。

超出阈值时，Additional Use Grant 仍**继续覆盖你 90 天**，这段时间用来谈许可，不会中断你的线上服务。

## 你**需要**商业许可

三种情况，每一种都对应 LICENSE 里的一条明确条款：

1. **你不再是 Small Entity**，但仍要把 Migo 嵌在自己的 App 里上线。
2. **你要把 Migo（或其衍生物）作为独立 SDK / 引擎 / 框架 / 运行时组件提供给第三方** —— 无论源码还是二进制、无论是否与其它代码打包在一起。
3. **你要提供托管、云或托管型小游戏运行时/引擎服务**，其核心运行时能力来自 Migo 或其衍生物。

第 2、3 条与规模无关：无论你多小，这两类用途都在 BSL 的 Use Limitation（"Competitive Offering"）范围内。

## 商业许可包含什么

- 解除上述限制的书面授权，按你实际的分发形态裁剪（嵌入式 / SDK 转售 / 托管服务）。
- 明确的版本与升级范围，以及可预期的 EOL 承诺。
- 可选：**设备适配与支持 SLA** —— 这是我们最有价值的东西。Migo 的正确性来自在真实机型与 ROM 上撞过的每一个坑；如果你的游戏在某台设备上表现异常，这条线是直接把那份积累用在你身上。
- 可选：兼容性认证、内容签名与分发、私有构建。

报价按分发形态、规模区间与支持等级确定，一事一议。首次沟通我们通常只需要三条信息：**你的分发形态、预计规模、目标机型范围**。

## 怎么开始

发一封邮件到 **licensing@minigame-labs.com**，说明上面三条即可。若涉及保密信息，可先索取 NDA。

安全问题请勿走本渠道，见 [SECURITY.md](SECURITY.md)。

## 相关文件

- [LICENSE](LICENSE) —— 具备约束力的完整条款，本页与其冲突时以 LICENSE 为准。
- [LEGAL.md](LEGAL.md) —— 许可、商标、第三方组件与测试内容的法律说明。
- [NOTICE](NOTICE) —— 第三方依赖及其许可证。

---

<a name="english"></a>

# Commercial Licensing

Migo is released under the [Business Source License 1.1](LICENSE). **Most users never need a commercial license.** This page states who does, who does not, and how to start the conversation.

## You do **not** need a commercial license

Granted directly by BSL 1.1 — free, no contact, nothing to sign:

- **Reading, auditing, building, testing, benchmarking, modifying and porting the source**, at any scale, by any entity. The BSL Terms grant non-production use to everyone; the size thresholds do **not** apply to it.
- **Shipping Migo inside your own app**, while you are a **Small Entity** as defined in the LICENSE:
  - total annual gross revenue of the entity and its affiliates, from all sources, of no more than **USD 1,000,000** in the most recently completed fiscal year; **and**
  - no more than **3,000,000 monthly active users** across all instances of your app in any single month of that year.
- **Developing, publishing and selling mini-games that run on Migo.** Revenue from the games themselves is unaffected.
- Academic research, teaching, personal and open-source projects.

If you cross a threshold, the Additional Use Grant keeps covering your production use for **90 days** so you can arrange a license without interrupting your service.

## You **do** need a commercial license

Three cases, each mapping to an explicit clause in the LICENSE:

1. **You are no longer a Small Entity** but still want to ship Migo inside your own app.
2. **You want to provide Migo (or a derivative) to third parties as a standalone SDK, engine, framework or runtime component** — source or binary, bundled with other code or not.
3. **You want to offer a hosted, cloud or managed mini-game runtime/engine service** whose core runtime functionality comes from Migo or a derivative.

Cases 2 and 3 are independent of size: they fall under the BSL Use Limitation ("Competitive Offering") no matter how small you are.

## What a commercial license covers

- Written grant lifting the above limits, scoped to how you actually distribute (embedded / SDK resale / hosted service).
- Defined version and upgrade scope, with a predictable EOL commitment.
- Optional: **device bring-up and a support SLA** — the most valuable thing we have. Migo is correct because of every trap hit on real handsets and real ROMs; this line points that accumulated work directly at your device matrix.
- Optional: compatibility certification, content signing and distribution, private builds.

Pricing is quoted per deal, based on distribution shape, scale band and support tier. A first conversation usually needs only three facts: **how you distribute, what scale you expect, and which devices you target.**

## How to start

Email **licensing@minigame-labs.com** with those three points. An NDA can be put in place first if needed.

Please do not use this channel for security reports — see [SECURITY.md](SECURITY.md).

## Related

- [LICENSE](LICENSE) — the binding terms; it governs wherever this page differs.
- [LEGAL.md](LEGAL.md) — licensing, trademark, third-party components and test content.
- [NOTICE](NOTICE) — third-party dependencies and their licenses.
