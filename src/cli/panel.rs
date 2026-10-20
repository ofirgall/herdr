use clap::Command;

use crate::api::schema::{EmptyParams, Method, PanelRestartProjection, Request};

use super::projection::{self, cli_projection, Leaf, ParseError};

cli_projection! {
    pub(crate) PanelListProjection for EmptyParams {}
}

const LEAVES: [Leaf; 2] = [
    Leaf {
        name: "list",
        about: "List server-owned panels",
        id: "cli:panel:list",
        arguments: PanelListProjection::arguments,
        method: |matches| PanelListProjection::from_matches(matches).map(Method::PanelList),
    },
    Leaf {
        name: "restart",
        about: "Restart a panel, for example after it exited",
        id: "cli:panel:restart",
        arguments: PanelRestartProjection::arguments,
        method: |matches| PanelRestartProjection::from_matches(matches).map(Method::PanelRestart),
    },
];

pub(crate) fn command() -> Command {
    Command::new("panel")
        .about("Inspect and restart sidebar panels over the socket API")
        .subcommand_required(true)
        .subcommands(LEAVES.iter().map(Leaf::command))
}

pub(crate) fn request(args: &[String]) -> Result<Request, ParseError> {
    projection::group_request(command(), &LEAVES, args)
}

pub(super) fn run_panel_command(args: &[String]) -> std::io::Result<i32> {
    super::run_json_request(request(args))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Request, ParseError> {
        request(&args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn panel_list_and_restart_build_their_requests() {
        let list = parse(&["list"]).unwrap();
        assert_eq!(list.method, Method::PanelList(EmptyParams {}));
        let restart = parse(&["restart", "prs"]).unwrap();
        assert_eq!(restart.id, "cli:panel:restart");
        assert_eq!(
            serde_json::to_value(&restart).unwrap()["params"],
            serde_json::json!({"id": "prs"})
        );
    }

    #[test]
    fn panel_rejects_invalid_grammar() {
        for values in [
            vec![],
            vec!["unknown"],
            vec!["list", "extra"],
            vec!["restart"],
            vec!["restart", "a", "b"],
        ] {
            assert!(parse(&values).is_err(), "argv: {values:?}");
        }
    }
}
