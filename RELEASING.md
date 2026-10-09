name: RELEASING

# 发版流程

仓库用的是 [cargo-dist](https://opensource.axo.dev/cargo-dist/)：推一个 `v*` tag，CI 自动出二进制、建 Release、推 Homebrew formula。**不需要手改任何文件。**

## 一次性准备

1. **建主仓库** `define9/mdp`（public），把本地内容推上去：

   ```bash
   git remote add origin git@github.com:define9/mdp.git
   git add -A && git commit -m "feat: mdp 0.1.0"
   git push -u origin main
   ```

2. **建 tap 仓库** `define9/homebrew-mdp`（public，**可以是完全空的仓库**，cargo-dist 会往里写 `Formula/mdp.rb`）：

   ```bash
   gh repo create define9/homebrew-mdp --public --description "Homebrew tap for mdp"
   ```

3. **建 PAT 并配成 secret**。formula 推送到另一个仓库需要写权限，`GITHUB_TOKEN` 跨仓库做不到：

   - GitHub → Settings → Developer settings → Personal access tokens → **Fine-grained**
   - Repository access：只选 `define9/homebrew-mdp`
   - Permissions：**Contents: Read and write**
   - 生成后加到 `define9/mdp` 的 secret：`Settings → Secrets and variables → Actions → New repository secret`，名字必须是 `HOMEBREW_TAP_TOKEN`

   > 不配这个 secret 的话：Release 和二进制正常发布，只有 `brew install define9/mdp/mdp` 不会自动更新。

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

CI 会依次完成：编译 arm64 + x86_64 → 打包 tar.xz → 生成 `install.sh` 和 `mdp.rb` → 建 GitHub Release（附 SHA256 和 attestation）→ 把 formula 推进 `define9/homebrew-mdp`。

## 发版后验证

```bash
brew tap define9/mdp
brew install mdp
mdp --version

curl -fsSL https://raw.githubusercontent.com/define9/mdp/main/install.sh | sh   # 非 brew 用户的路径
```

顺手检查 Release 页面：两个 `.tar.xz` 都在、`sha256.sum` 完整、formula 的 tag 和版本对得上。

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
