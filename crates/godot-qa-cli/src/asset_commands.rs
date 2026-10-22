use crate::{AssetArgs, AssetCommands, Cli, runtime_config};
use godot_qa_core::results::CommandResult;
use godot_qa_scenario::asset_trial;

pub(crate) fn asset_result(cli: &Cli, args: &AssetArgs) -> CommandResult {
    let config = runtime_config(cli);
    match &args.command {
        AssetCommands::Try { manifest } => asset_trial::run_asset_try(&config, manifest),
        AssetCommands::Status { trial_id } => {
            asset_trial::run_asset_status(&config, trial_id.as_deref())
        }
        AssetCommands::Keep { trial_id } => asset_trial::run_asset_keep(&config, trial_id),
        AssetCommands::Discard { trial_id } => asset_trial::run_asset_discard(&config, trial_id),
    }
}
