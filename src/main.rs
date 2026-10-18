use std::env;
use std::process::ExitCode;

use magickeyboard::{FnMode, Iso, Options, SwapOptCmd};

fn main() -> ExitCode {
    match parse_args(env::args().skip(1)) {
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
        Ok(Command::Help) => {
            println!("{HELP}");
            ExitCode::SUCCESS
        }
        Ok(Command::Status) => match magickeyboard::status() {
            Ok(text) => {
                print!("{text}");
                ExitCode::SUCCESS
            }
            Err(message) => {
                eprintln!("{message}");
                ExitCode::from(1)
            }
        },
        Ok(Command::Listen(options)) => match magickeyboard::listen(&options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("{message}");
                ExitCode::from(1)
            }
        },
        Ok(Command::Raw(raw)) => match run_raw(raw) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("{message}");
                ExitCode::from(1)
            }
        },
    }
}

fn run_raw(raw: RawArgs) -> Result<(), String> {
    if raw.list {
        print!("{}", magickeyboard::raw_status()?);
        if raw.seconds.is_none() {
            return Ok(());
        }
    }
    magickeyboard::listen_raw(raw.seconds)
}

enum Command {
    Status,
    Listen(Options),
    Raw(RawArgs),
    Help,
}

struct RawArgs {
    list: bool,
    seconds: Option<u64>,
}

const HELP: &str = "\
magickeyboard status
magickeyboard listen [--fn-mode MODE] [--swap-opt-cmd [both|left]] [--swap-ctrl-cmd] [--swap-fn-leftctrl] [--iso | --no-iso]
magickeyboard raw [--list] [--seconds N]

status   print the connected Magic Keyboard and the idle report
listen   print key, Fn, and consumer events until Ctrl+C
raw      print Raw Input scan codes and HID reports until Ctrl+C
--list   print the Apple devices Raw Input can see, then exit
--seconds N
         stop raw after N seconds; 0 registers and exits

--fn-mode auto | 3            same as fkeyslast on these keyboards (default)
--fn-mode fkeyslast | media | 1
                              F-row is brightness and media until Fn is held
--fn-mode fkeysfirst | fkeys | 2
                              F-row stays F1-F12 until Fn is held
--fn-mode fkeysdisabled | 4   F-row stays F1-F12; arrows still follow Fn
--fn-mode disabled | off | 0  no Fn translation, including arrows
--swap-opt-cmd [both|left]    swap Option and Command (both sides, or left only)
--swap-ctrl-cmd               swap Control and Command
--swap-fn-leftctrl            swap Fn and left Control
--iso                         swap ` and the non-US key
--no-iso                      keep ` and the non-US key
";

fn parse_args(args: impl Iterator<Item = String>) -> Result<Command, String> {
    let mut command: Option<Command> = None;
    let mut options = Options::default();
    let mut raw_list = false;
    let mut raw_seconds: Option<u64> = None;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "status" => set_command(&mut command, Command::Status)?,
            "listen" => set_command(&mut command, Command::Listen(options))?,
            "raw" => set_command(
                &mut command,
                Command::Raw(RawArgs {
                    list: false,
                    seconds: None,
                }),
            )?,
            "--list" => raw_list = true,
            "--seconds" => {
                let value = args
                    .next()
                    .ok_or_else(|| "missing value for --seconds".to_string())?;
                raw_seconds = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| format!("unknown seconds {value}"))?,
                );
            }
            "help" | "--help" | "-h" => return Ok(Command::Help),
            "--swap-opt-cmd" => {
                options.swap_opt_cmd = match args.peek().map(String::as_str) {
                    Some("both") => {
                        args.next();
                        SwapOptCmd::Both
                    }
                    Some("left") => {
                        args.next();
                        SwapOptCmd::LeftOnly
                    }
                    _ => SwapOptCmd::Both,
                };
            }
            "--swap-ctrl-cmd" => options.swap_ctrl_cmd = true,
            "--swap-fn-leftctrl" => options.swap_fn_leftctrl = true,
            "--iso" => options.iso = Iso::On,
            "--no-iso" => options.iso = Iso::Off,
            "--fn-mode" => {
                let value = args
                    .next()
                    .ok_or_else(|| "missing value for --fn-mode".to_string())?;
                options.fn_mode = parse_mode(&value)?;
            }
            other => return Err(format!("unknown argument {other}\n{HELP}")),
        }
    }
    if matches!(command, Some(Command::Listen(_))) {
        command = Some(Command::Listen(options));
    }
    if matches!(command, Some(Command::Raw(_))) {
        command = Some(Command::Raw(RawArgs {
            list: raw_list,
            seconds: raw_seconds,
        }));
    } else if raw_list || raw_seconds.is_some() {
        return Err(format!("--list and --seconds belong to raw\n{HELP}"));
    }
    Ok(command.unwrap_or(Command::Status))
}

fn set_command(slot: &mut Option<Command>, command: Command) -> Result<(), String> {
    if slot.is_some() {
        return Err("pass one of status, listen, or raw".to_string());
    }
    *slot = Some(command);
    Ok(())
}

fn parse_mode(value: &str) -> Result<FnMode, String> {
    match value {
        "0" | "disabled" | "off" => Ok(FnMode::Disabled),
        "1" | "fkeyslast" | "media" => Ok(FnMode::FkeysLast),
        "2" | "fkeysfirst" | "fkeys" => Ok(FnMode::FkeysFirst),
        "3" | "auto" => Ok(FnMode::Auto),
        "4" | "fkeysdisabled" => Ok(FnMode::FkeysDisabled),
        other => Err(format!("unknown fn mode {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_accept_linux_names_and_the_old_aliases() {
        let Command::Listen(options) = parse_args(
            [
                "--fn-mode",
                "4",
                "--swap-opt-cmd",
                "left",
                "--swap-ctrl-cmd",
                "--swap-fn-leftctrl",
                "listen",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap() else {
            panic!("listen");
        };
        assert_eq!(options.fn_mode, FnMode::FkeysDisabled);
        assert_eq!(options.swap_opt_cmd, SwapOptCmd::LeftOnly);
        assert!(options.swap_ctrl_cmd);
        assert!(options.swap_fn_leftctrl);

        let Command::Listen(options) = parse_args(
            ["listen", "--fn-mode", "media"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap() else {
            panic!("listen");
        };
        assert_eq!(options.fn_mode, FnMode::FkeysLast);
        assert_eq!(options.swap_opt_cmd, SwapOptCmd::Off);
    }

    #[test]
    fn raw_list_does_not_listen() {
        let Command::Raw(raw) = parse_args(
            ["--seconds", "0", "raw", "--list"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap() else {
            panic!("raw");
        };
        assert!(raw.list);
        assert_eq!(raw.seconds, Some(0));
    }
}
