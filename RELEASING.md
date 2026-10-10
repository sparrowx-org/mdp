# RELEASING

# 发版流程

仓库用的是 [cargo-dist](https://opensource.axo.dev/cargo-dist/)：推一个 `v*` tag，CI 自动出二进制、建 Release、推 Homebrew formula。**不需要手改任何文件。**

## 组织结构

所有软件都在 GitHub 组织 `sparrowx` 下（域名 sparrowx.org）：

```
sparrowx-org/mdp              软件源码仓（每个软件一个仓）
sparrowx-org/homebrew-tap     共用 tap 仓，Formula/ 里放所有软件的 formula
```

发布一个软件时，cargo-dist 只往 `homebrew-tap` 写自己那一个 `Formula/<name>.rb`，不会碰别人的文件，多个软件可以共用一个 tap。

## 一次性准备

1. **主仓库** `sparrowx-org/mdp`（public）：源码仓。

2. **tap 仓库** `sparrowx-org/homebrew-tap`（public，**可以是完全空的仓库**，cargo-dist 会往里写 `Formula/mdp.rb`）：

   ```bash
   gh repo create sparrowx-org/homebrew-tap --public --description "Homebrew tap for SparrowX tools"
   ```

3. **建 PAT 并配成 secret**。formula 要推送到另一个仓库，需要写权限，`GITHUB_TOKEN` 跨仓库做不到：

   - GitHub → Settings → Developer settings → Personal access tokens → **Fine-grained**
   - Resource owner：**SparrowX**（选组织，不要选个人账号，否则以后别人发版推不动）
   - Repository access：只选 `homebrew-tap`
   - Permissions：**Contents: Read and write**
   - 有效期给长一点（发版不频繁，过期后所有软件会同时停更）
   - 生成后加到**组织级** secret：`sparrowx-org` → Settings → Secrets and variables → Actions → New repository secret，名字必须是 `HOMEBREW_TAP_TOKEN`，**Repository access 里勾上 `mdp`**（以及以后每个软件仓）

   > 组织级 secret 默认不对任何仓生效，**新仓必须去这个页面重新勾一次**，漏勾的话 CI 里 `token` 为空，formula 推不上去。

   > 不配这个 secret 的话：Release 和二进制正常发布，只有 `brew install sparrowx-org/tap/mdp` 不会自动更新。

4. **确认 `workflow` 有权限**：`Settings → Actions → General → Workflow permissions → Read and write permissions`
   （tag push 触发 Release 需要写权限；默认可能是只读。）

## 发一个版本

```bash
# 1. 确认干净
cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check

# 2. 改版本号（两处 Cargo.toml / Cargo.lock 一起）
cargo set-version --workspace 0.1.0     # 或手动改
git commit -am "chore: v0.1.0"

# 3. 打 tag —— 这一步就是触发器
git tag -a v0.1.0 -m "v0.1.0"
git push origin main && git push origin v0.1.0
```

CI 会依次完成：编译 arm64 + x86_64 → 打包 tar.xz → 生成 `install.sh` 和 `mdp.rb` → 建 GitHub Release（附 SHA256 和 attestation）→ 把 formula 推进 `sparrowx-org/homebrew-tap`。

## 发版后验证

```bash
brew tap sparrowx-org/tap
brew install sparrowx-org/tap/mdp
mdp --version

curl -fsSL https://raw.githubusercontent.com/sparrowx-org/mdp/main/install.sh | sh   # 非 brew 用户的路径
```

顺手检查 Release 页面：两个 `.tar.xz` 都在、`sha256.sum` 完整、formula 的 tag 和版本对得上。

## 新软件接进这个 tap

1. 源码仓建在 `sparrowx-org/<tool>`，照这个仓的 `dist-workspace.toml` 抄一份，把 `tap` 改成 `"sparrowx-org/homebrew-tap"`，`targets` 按平台调整。
2. `cargo dist generate` 生成对应 workflow。
3. 在组织 secret `HOMEBREW_TAP_TOKEN` 的 Repository access 里勾上新的 `sparrowx-org/<tool>` 仓（**组织级 secret 不会自动对新仓生效，漏勾就推不动 formula**）。
4. 打 tag 发布，`sparrowx-org/homebrew-tap` 里就多出一个 `Formula/<tool>.rb`。

## 预发布

带预发布后缀的 tag（`v0.2.0-beta.1`）会被标成 prerelease，**不会**推 Homebrew formula：

```bash
git tag -a v0.2.0-beta.1 -m "beta" && git push origin v0.2.0-beta.1
```

想让预发布也进 tap，在 `dist-workspace.toml` 里加 `publish-prereleases = true`（默认是 `false`）。

## 改了构建配置之后

`dist-workspace.toml` 或 `Cargo.toml` 的 `[dist]` 段改动后，跑一次 `cargo dist generate` 让 `.github/workflows/release.yml` 与之同步；这个文件是自动生成的，**手改会在下次 generate 时被覆盖**。

## 回滚

Release 是 tag 驱动的，没有"重新发一版"按钮：改代码 → 发 `v0.1.1`。要撤回已经发出去的版本，在 GitHub Release 页面删掉对应 release 和 tag 即可。