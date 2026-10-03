// Shared rules of the dice duel (examples/stories/test_script.ron, M8-05).
// A module: imported by name from the story's scripts with
//     import "dice" as dice;
// It only reads and writes story variables through the script API, so
// saves, rollback and replays see everything it does.

// Starts a round: the house rolls its die (the VM's seeded RNG).
fn new_round() {
    set_var("round", get_var("round") + 1);
    set_var("roll", rand_int(1, 6));
}

// The player bets that their die will be higher or lower than the house's.
// Returns the branch of the script node: "win" or "lose" (a tie loses).
fn bet(side) {
    let house = get_var("roll");
    let mine = rand_int(1, 6);
    let won = if side == "higher" { mine > house } else { mine < house };
    set_var("history", get_var("history") + [`${side} ${mine} vs ${house}`]);
    if won {
        set_var("gold", get_var("gold") + 3);
        set_var("wins", get_var("wins") + 1);
        "win"
    } else {
        set_var("gold", get_var("gold") - 2);
        "lose"
    }
}
