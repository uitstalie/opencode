# opencode API Key 管理机制分析

> 分析时间：2026-06-15
> 源码范围：`source-code/opencode/packages/` 下所有 `.ts` 文件，`specs/` 下所有 `.md` 文件
> 构建版本：opencode dev 分支

---

## 1. 关键文件路径与行号

### Auth 核心模块（存储与读写）

| 文件 | 行号 | 作用 |
|------|------|------|
| `packages/opencode/src/auth/index.ts` | 1-99 | Auth 服务：key 的存储、读取、写入、删除。**全文件为关键文件** |
| `packages/opencode/src/auth/index.ts` | 10 | `auth.json` 存储路径：`path.join(Global.Path.data, "auth.json")` |
| `packages/opencode/src/auth/index.ts` | 14-21 | `Oauth` 类型：`refresh` + `access` + `expires` — OAuth token 以明文存储 |
| `packages/opencode/src/auth/index.ts` | 23-27 | `Api` 类型：`key` (Schema.String) — **API key 以明文 `Schema.String` 存储，无加密** |
| `packages/opencode/src/auth/index.ts` | 29-33 | `WellKnown` 类型：`key` + `token` — 用于 `.well-known/opencode` 认证 |
| `packages/opencode/src/auth/index.ts` | 59-62 | `OPENCODE_AUTH_CONTENT` 环境变量可绕过文件直接注入 auth 数据 |
| `packages/opencode/src/auth/index.ts` | 73-81 | `auth.set()` 写入 `auth.json`，使用 `0o600` 权限（仅所有者读写） |

### CLI "connect" 命令（用户登录/连接 provider）

| 文件 | 行号 | 作用 |
|------|------|------|
| `packages/opencode/src/cli/cmd/providers.ts` | 240-246 | 顶层命令 `opencode providers`，**别名 `opencode auth`** |
| `packages/opencode/src/cli/cmd/providers.ts` | 299-489 | `opencode providers login` — 用户登录 provider，输入 API key 或 OAuth |
| `packages/opencode/src/cli/cmd/providers.ts` | 95-170 | OAuth 流程：跳转浏览器 → 回调 → 保存 refresh/access token |
| `packages/opencode/src/cli/cmd/providers.ts` | 172-207 | API key 流程：`Prompt.password()` 输入 → `authSvc.set(provider, { type: "api", key: apiKey })` |
| `packages/opencode/src/cli/cmd/providers.ts` | 480-485 | **用户输入的 key 直接以明文写入 auth.json，无任何加密步骤** |
| `packages/opencode/src/cli/cmd/providers.ts` | 491-533 | `opencode providers logout` — 删除 credential |

### Provider 内部 key 使用（解密？→ 实际直接用明文）

| 文件 | 行号 | 作用 |
|------|------|------|
| `packages/opencode/src/provider/provider.ts` | 1484-1494 | 从 auth.json 加载 API key：`provider.key` → `mergeProvider(id, { source: "api", key: provider.key })` |
| `packages/opencode/src/provider/provider.ts` | 1360 | 插件 provider 模型加载时传入 `auth: pluginAuth` — 即原始 Auth.Info |
| `packages/opencode/src/provider/provider.ts` | 1497-1516 | 插件 `auth.loader` 调用：`plugin.auth.loader(auth, toPublicInfo(database[...]))` — loader 可拿到完整 Auth |
| `packages/opencode/src/provider/provider.ts` | 1669 | SDK 调用时注入 key：`if (options["apiKey"] === undefined && provider.key) options["apiKey"] = provider.key` |
| `packages/opencode/src/provider/provider.ts` | 1290-1317 | Provider 层初始化：`const auth = yield* Auth.Service` → `dep.auth = (id) => auth.get(id)` |

### Provider Info 类型定义

| 文件 | 行号 | 作用 |
|------|------|------|
| `packages/opencode/src/provider/provider.ts` | 1035-1044 | Provider.Info 有 `key?: string` 字段 — **明文 key 直接暴露在 Provider 对象上** |
| `packages/core/src/provider.ts` | 47-68 | ProviderV2.Info（核心层）— **不含 key 字段**，key 是 packages/opencode 层加的 |
| `packages/sdk/js/src/v2/gen/types.gen.ts` | 2133-2145 | SDK Provider 类型：`key?: string` — **API key 在 SDK 层面也是明文传输** |
| `packages/sdk/js/src/v2/gen/types.gen.ts` | 116-122 | ApiAuth 类型：`{ type: "api", key: string, metadata? }` |
| `packages/sdk/js/src/v2/gen/types.gen.ts` | 130 | Auth 联合类型：`OAuth | ApiAuth | WellKnownAuth` |

### 插件相关

| 文件 | 行号 | 作用 |
|------|------|------|
| `packages/plugin/src/index.ts` | 20-24 | `ProviderContext` 类型：`{ source, info: Provider, options }` — info 含 `key?: string` |
| `packages/plugin/src/index.ts` | 88-90 | `AuthHook.loader` 签名：`(auth: () => Promise<Auth>, provider: Provider) => Promise<Record<string, any>>` |
| `packages/plugin/src/index.ts` | 247-256 | `chat.params` / `chat.headers` 钩子：接收 `provider: ProviderContext` — 其中 `info` 含 key |
| `packages/plugin/src/index.ts` | 297 | `experimental.provider.small_model`：接收 `provider: ProviderV2` |

### Specs 文档

| 文件 | 说明 |
|------|------|
| `specs/v2/provider-model.md` | Provider / Model 架构设计文档，提到 plugin hooks 但未提及密钥加密 |
| `specs/v2/provider-policy.md` | Provider 策略（允许/拒绝），与密钥存储无关 |

---

## 2. Key 加密/解密机制概述

### 结论：**没有加密机制**

经过全量搜索：

1. **代码库中不存在 `encrypt`、`decrypt`、`cipher`、`aes` 等关键词**（与 API key 存储相关）
2. Auth 模块将所有 API key 以 **`Schema.String`（明文）** 类型直接写入 `auth.json`
3. 唯一的保护是文件系统权限 `0o600`（仅文件所有者可读写）
4. **没有 keychain、keytar、vault、系统密钥链等安全存储方案的调用**

### 数据流

```
用户输入 (Prompt.password)
  ↓ (明文)
authSvc.set(provider, { type: "api", key: "sk-xxx" })
  ↓ (明文写入 JSON)
~/.config/opencode/data/auth.json  (0o600)
  ↓ (明文读取)
Auth.Service → Provider.Info.key
  ↓ (明文传递)
AI SDK: createProvider({ apiKey: provider.key })
```

### 三种 Auth 类型

| 类型 | 存储字段 | 说明 |
|------|----------|------|
| `oauth` | `refresh`, `access`, `expires` | OAuth 2.0 token，均明文存储 |
| `api` | `key` (string), `metadata?` | API key，明文存储 |
| `wellknown` | `key` (string), `token` (string) | 用于 `.well-known/opencode` 自动化认证 |

---

## 3. 插件层面能否获取解密后的 key？

### 答案：**可以，且几乎无阻力**

由于 key 本身就是明文传输，插件可以通过以下途径获取 API key：

#### 途径 A：`AuthHook.loader` 回调（最直接）

```ts
// 在插件中定义
export const plugin = async (input) => ({
  auth: {
    provider: "my-provider",
    loader: async (auth, provider) => {
      const stored = await auth()  // 返回 Auth (OAuth | ApiAuth | WellKnownAuth)
      // stored.key 就是明文 API key（ApiAuth 类型）
      console.log(stored.key)  // 直接拿到
      return { /* options */ }
    },
    methods: [/* ... */],
  }
})
```

**源码位置**：`packages/opencode/src/provider/provider.ts:1508-1510`
```ts
plugin.auth!.loader!(
  () => bridge.promise(auth.get(providerID).pipe(Effect.orDie)) as any,
  toPublicInfo(database[plugin.auth!.provider]),
)
```

#### 途径 B：`chat.params` / `chat.headers` / `chat.message` 钩子

```ts
// ProviderContext.info.key 包含明文 API key
{
  "chat.params": (input, output) => {
    const key = input.provider.info.key  // 直接获取
    // 或 input.provider.options 中的间接配置
  }
}
```

**源码位置**：`packages/plugin/src/index.ts:20-24` — `ProviderContext` 定义。

#### 途径 C：Provider list API（SDK 调用）

```ts
// 通过 SDK 获取 provider 列表
const providers = await client.provider.list()
providers[0].key  // SDK Provider 类型有 key?: string 字段
```

**源码位置**：`packages/sdk/js/src/v2/gen/types.gen.ts:2133-2145` — Provider 类型含 `key?: string`。

---

## 4. 替代方案（如果不想暴露明文 key）

由于 opencode 目前没有内置加密机制，如果需要保护 API key，可以从以下方向考虑：

### 方案 1：环境变量（已有支持，推荐）
- Provider 配置中 `env: ["MY_SECRET_KEY"]` → 从环境变量读取
- 不经过 auth.json，不存储到磁盘
- **局限**：需要外部管理环境变量（如 shell profile、systemd、docker secrets）

### 方案 2：OPENCODE_AUTH_CONTENT 环境变量（已有支持）
- 设置 `OPENCODE_AUTH_CONTENT='{"my-provider":{"type":"api","key":"sk-xxx"}}'`
- 完全绕过 auth.json 文件
- **局限**：key 仍以明文出现在进程环境变量中

### 方案 3：系统密钥链集成（需要修改 opencode 源码）
- 集成 `keytar`、`credential-store` 等 Node 包
- 修改 `packages/opencode/src/auth/index.ts` 的 `set()`/`get()`/`all()` 方法
- 将 key 存储到 macOS Keychain / Windows Credential Manager / Linux Secret Service
- **局限**：需要跨平台适配，目前源码无此支持

### 方案 4：代理/网关层注入（架构层方案）
- 在请求到达 AI provider 之前由 API Gateway 注入 API key
- opencode 完全不持有真实 key，只持有网关认证 token
- 类似 Cloudflare AI Gateway、自建 nginx/litellm 代理的用法

---

## 5. 结论摘要

| 维度 | 结论 |
|------|------|
| 加密机制 | **无** — 全链路明文 |
| 存储安全 | 仅文件系统权限 `0o600`，无加密 |
| 传输安全 | 进程内明文传递；Provider list API 返回明文 key |
| 插件是否可获取 key | **是**，至少 3 种途径可以直接拿到明文 key |
| 可改进方向 | 环境变量已支持（`env` 字段 + `OPENCODE_AUTH_CONTENT`）；系统密钥链需改源码 |

**根本原因**：opencode 的设计哲学是"工具链本地安全"（依赖 OS 用户隔离 + 文件权限），而不是"数据加密安全"。对于客户端工具来说这是常见做法（与 git credential、npmrc 等类似），但如果有更高安全需求，建议使用环境变量方案或网关代理方案。
