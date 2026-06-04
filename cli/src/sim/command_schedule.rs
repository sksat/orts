//! 時刻指定コマンドシーケンス（config transport）。
//!
//! `orts.toml` の `[[command]]` を時刻順のキューにし、シミュレーション時刻が
//! 進むにつれて該当 tick の衛星コントローラへ配送する。host が「どの
//! コマンドをどの tick に配るか」を確定するので **決定論的**で、同じ config は
//! 同じ結果を再現する（WebSocket 対話 transport と対照的）。
//!
//! これは host 所有の transport-agnostic キューの一実装（adapter）。
//! 配送先 FSW から見れば、WebSocket 由来か config 由来かは区別されない。

use orts::plugin::Message;

/// 1 件の時刻指定コマンド。
#[derive(Debug, Clone)]
pub struct ScheduledCommand {
    /// 配送するシミュレーション時刻 \[s\]。
    pub t: f64,
    /// 配送先衛星のインデックス（`SimParams::satellites` 内の位置）。
    pub sat_index: usize,
    /// 配送するメッセージ。`host_seq` / `deliver_tick` は配送時に host が
    /// 上書きする（ここではテンプレート）。
    pub message: Message,
}

/// 時刻順に整列したコマンドキュー。カーソルで「未配送の先頭」を追う。
pub struct CommandSchedule {
    commands: Vec<ScheduledCommand>,
    cursor: usize,
}

impl CommandSchedule {
    /// コマンド列から構築する。`t` で安定ソートするので、同時刻のコマンドは
    /// 宣言順を保つ（決定論）。
    pub fn new(mut commands: Vec<ScheduledCommand>) -> Self {
        commands.sort_by(|a, b| a.t.total_cmp(&b.t));
        Self {
            commands,
            cursor: 0,
        }
    }

    /// 時刻 `t_due` までに配送すべき（`t <= t_due`）コマンドを返し、カーソルを
    /// 進める。各コマンドはちょうど 1 回だけ返る。
    ///
    /// run ループは各 tick の終端時刻（`t + dt`）を渡し、その tick で
    /// 配送が確定したコマンドを得る。
    pub fn drain_due(&mut self, t_due: f64) -> &[ScheduledCommand] {
        let start = self.cursor;
        while self.cursor < self.commands.len() && self.commands[self.cursor].t <= t_due {
            self.cursor += 1;
        }
        &self.commands[start..self.cursor]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orts::plugin::{NodeId, Payload};

    fn cmd(t: f64, sat_index: usize, kind: &str) -> ScheduledCommand {
        ScheduledCommand {
            t,
            sat_index,
            message: Message {
                src: NodeId::Ground,
                dst: NodeId::Satellite(sat_index as u32),
                kind: kind.to_string(),
                host_seq: 0,
                deliver_tick: 0,
                payload: Payload::KeyValue(vec![]),
            },
        }
    }

    #[test]
    fn drain_due_yields_each_command_once_in_time_order() {
        let mut sched = CommandSchedule::new(vec![
            cmd(300.0, 0, "c.300"),
            cmd(100.0, 0, "c.100"),
            cmd(200.0, 0, "c.200"),
        ]);

        // Nothing due before t=100.
        assert!(sched.drain_due(50.0).is_empty());
        // t<=150 → only the t=100 command.
        let due = sched.drain_due(150.0);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].message.kind, "c.100");
        // t<=300 → the remaining two, in time order, not re-yielding c.100.
        let due: Vec<&str> = sched
            .drain_due(300.0)
            .iter()
            .map(|c| c.message.kind.as_str())
            .collect();
        assert_eq!(due, vec!["c.200", "c.300"]);
        // Exhausted.
        assert!(sched.drain_due(1000.0).is_empty());
    }

    #[test]
    fn equal_time_commands_keep_declaration_order() {
        let mut sched = CommandSchedule::new(vec![
            cmd(100.0, 0, "first"),
            cmd(100.0, 1, "second"),
            cmd(100.0, 0, "third"),
        ]);
        let order: Vec<&str> = sched
            .drain_due(100.0)
            .iter()
            .map(|c| c.message.kind.as_str())
            .collect();
        assert_eq!(order, vec!["first", "second", "third"]);
    }

    #[test]
    fn boundary_is_inclusive() {
        let mut sched = CommandSchedule::new(vec![cmd(100.0, 0, "at-100")]);
        // exactly t == t_due delivers.
        assert_eq!(sched.drain_due(100.0).len(), 1);
    }

    #[test]
    fn empty_schedule() {
        let mut sched = CommandSchedule::new(vec![]);
        assert!(sched.drain_due(1e9).is_empty());
    }
}
