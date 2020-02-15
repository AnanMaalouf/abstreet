mod abtest;
mod challenges;
mod colors;
mod common;
mod debug;
mod edit;
mod game;
mod helpers;
mod managed;
mod mission;
mod obj_actions;
mod options;
mod pregame;
mod render;
mod sandbox;
mod ui;

use crate::ui::Flags;
use abstutil::CmdArgs;
use sim::SimFlags;

fn main() {
    let mut args = CmdArgs::new();

    // TODO Lift this out of the game crate entirely.
    if args.enabled("--prebake") {
        challenges::prebake();
        return;
    }

    let mut flags = Flags {
        sim_flags: SimFlags::from_args(&mut args),
        kml: args.optional("--kml"),
        draw_lane_markings: !args.enabled("--dont_draw_lane_markings"),
        num_agents: args.optional_parse("--num_agents", |s| s.parse()),
    };
    // TODO tmp
    flags.sim_flags.rng_seed = Some(42);
    let mut opts = options::Options::default();
    if args.enabled("--dev") {
        opts.dev = true;
        flags.sim_flags.rng_seed = Some(42);
    }
    if let Some(x) = args.optional("--color_scheme") {
        opts.color_scheme = Some(format!("../data/system/{}", x));
    }
    let mut settings = ezgui::Settings::new("A/B Street", "../data/system/fonts");
    if args.enabled("--enable_profiler") {
        settings.enable_profiling();
    }
    if args.enabled("--dump_raw_events") {
        settings.dump_raw_events();
    }
    if let Some(n) = args.optional_parse("--font_size", |s| s.parse::<usize>()) {
        settings.default_font_size(n);
    }

    let mut mode = sandbox::GameplayMode::Freeform;
    if let Some(x) = Some("trafficsig/tut1") {
        //args.optional("--challenge") {
        let mut aliases = Vec::new();
        'OUTER: for (_, stages) in challenges::all_challenges(true) {
            for challenge in stages {
                if challenge.alias == x {
                    mode = challenge.gameplay;
                    flags.sim_flags.load = challenge.map_path;
                    break 'OUTER;
                } else {
                    aliases.push(challenge.alias);
                }
            }
        }
        if mode == sandbox::GameplayMode::Freeform {
            panic!(
                "Don't know --challenge={}. Choices: {}",
                x,
                aliases.join(", ")
            );
        }
    }
    if let Some(n) = args.optional_parse("--tutorial", |s| s.parse::<usize>()) {
        mode = sandbox::GameplayMode::Tutorial(n - 1);
    }

    args.done();

    ezgui::run(settings, |ctx| game::Game::new(flags, opts, mode, ctx));
}
