# CRB Mining Guide

## 快速开始

### 环境要求

- Node.js v20+
- Solana CLI（可选，用于创建钱包）
- 一个有 SOL 的 Solana 钱包（建议 0.5 SOL 以上）

### 1. 克隆仓库

```bash
git clone https://github.com/Gus567897/poi-token.git
cd poi-token/miner
npm install
```

### 2. 创建矿工钱包

```bash
solana-keygen new -o miner-keypair.json
solana address -k miner-keypair.json
```

往这个地址充入 SOL 作为交易手续费。

### 3. 配置环境变量

```bash
# 必填：矿工钱包路径
export KEYPAIR=/path/to/miner-keypair.json

# 可选：自定义 RPC（默认 https://solana-rpc.publicnode.com）
export RPC_URL=https://your-rpc-endpoint.com

# 可选：指定接收 CRB 代币的钱包（默认与矿工钱包相同）
export RECIPIENT=<recipient-wallet-address>
```

### 4. 开始挖矿

```bash
npx ts-node --transpile-only mainnet-miner.ts
```

后台运行：

```bash
nohup npx ts-node --transpile-only mainnet-miner.ts >> miner.log 2>&1 &
```

## 挖矿机制

### 每个 Epoch 的流程

1. 读取链上状态，获取当前 epoch、难度、challenge seed
2. 从 challenge seed 推导出必须包含的单词
3. 生成包含这些单词的自然语言文本（256-800 字节）
4. 找到一个 nonce 使得 `keccak256(seed | miner_key | text | "||" | nonce)` 满足难度要求
5. 提交 solution 到链上
6. Epoch 结束后，任何人都可以推进到下一个 epoch（permissionless crank）
7. Claim 奖励到 VestingAccount（锁定状态）
8. 锁定的代币在 30 天内线性释放，随时可以 withdraw 已解锁的部分

### 关键参数

| 参数 | 值 |
|------|-----|
| Epoch 时长 | 600 秒（10 分钟） |
| 目标 Solutions | 每 epoch 50 个 |
| 难度范围 | 4 - 250 |
| 初始奖励 | 25,000 CRB / solution |
| 减半间隔 | 每 2,000,000 个 solutions |
| Vesting | 30 天线性释放 |
| 每 epoch 每矿工 | 最多 1 个 solution |

### 难度调整

每个 epoch 结束时根据 solution 数量调整：
- solution 太多 → 难度上升
- solution 太少 → 难度下降
- 零 solution → 最大幅度下降（-5）

### Vesting（锁仓释放）

- Claim 的奖励进入 VestingAccount，锁定状态
- 30 天内线性解锁
- 随时可以 withdraw 已解锁的代币
- 停止挖矿后，已锁定的代币继续正常释放
- 新的 claim 会叠加到现有锁仓余额上

## 合约信息

| 项目 | 值 |
|------|-----|
| Program ID | `AcTXBfHAJgwt1sTn3DvTSKiiCKgShzGEZzq2zQrs5BnG` |
| Token Mint | `7HYtCPSMAUAujsSesBSyccK2hsdTfFW2sX63SoaedJh3` |
| Decimals | 3 |
| Max Supply | 100,000,000,000 CRB |

## 常见问题

### RPC 选择

公共 RPC 有速率限制，建议使用付费 RPC：
- [Helius](https://dev.helius.xyz) — 免费版每月 100 万请求
- [QuickNode](https://quicknode.com)
- [Alchemy](https://alchemy.com)

### 手续费

每次提交 solution 大约消耗 0.000005 SOL，加上 priority fee。0.5 SOL 足够挖很久。

### 多矿工

可以用不同钱包运行多个矿工实例，每个矿工每 epoch 最多提交 1 个 solution。

### Advance Epoch

Epoch 结束后需要有人调用 `advance_epoch` 推进到下一轮。这是 permissionless 的，矿工程序会自动处理。即使你的矿工没有推进，其他矿工也会推进。
