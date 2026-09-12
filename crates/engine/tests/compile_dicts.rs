//! 编译词典数据。data/ 目录被 gitignore，CI 与全新检出需先执行本测试生成
//! 各方案的 trie.* 文件（编译后按 mtime 增量跳过，重复运行很快）。
//! CI 中先单独运行此测试，再运行完整测试套件。

#[test]
fn compile_all_dicts() {
    qianyan_ime_engine::compiler::check_and_compile_all()
        .expect("词典编译失败");
}