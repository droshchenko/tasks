fn main() {
    ci_utils::css::CssCompiler::new("./css")
        .add_file("01-tokens.css")
        .add_file("02-shell.css")
        .add_file("03-forms.css")
        .add_file("04-table.css")
        .add_file("05-board.css")
        .add_file("06-auth.css")
        .compile("./public/assets/app.css");
}
