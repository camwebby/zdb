//! EXPLAIN output → a flat list of plan nodes with depth and self cost, for the tree view.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct PlanNode {
    pub depth: usize,
    pub label: String,
    pub detail: Option<String>,
    /// Cost (or time, for ANALYZE) spent in this node excluding children.
    pub self_cost: f64,
    pub total_cost: f64,
    pub rows: Option<f64>,
    pub actual_ms: Option<f64>,
    pub last_child: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub nodes: Vec<PlanNode>,
    pub footer: Vec<String>,
    pub analyze: bool,
}

pub fn parse_pg_json(text: &str, analyze: bool) -> Result<Plan, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("could not read plan: {e}"))?;
    let top = v.get(0).ok_or("empty plan")?;
    let root = top.get("Plan").ok_or("no Plan in output")?;
    let mut plan = Plan { analyze, ..Default::default() };
    walk_pg(root, 0, true, analyze, &mut plan.nodes);
    for (k, label) in [("Planning Time", "planning"), ("Execution Time", "execution")] {
        if let Some(t) = top.get(k).and_then(|t| t.as_f64()) {
            plan.footer.push(format!("{label} {t:.3} ms"));
        }
    }
    Ok(plan)
}

fn walk_pg(n: &Value, depth: usize, last: bool, analyze: bool, out: &mut Vec<PlanNode>) {
    let s = |k: &str| n.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let f = |k: &str| n.get(k).and_then(|v| v.as_f64());
    let mut label = s("Node Type").unwrap_or_else(|| "?".into());
    if let Some(j) = s("Join Type").filter(|j| j != "Inner") {
        label = label.replace(" Join", &format!(" {j} Join")).replace("Nested Loop", &format!("Nested Loop {j}"));
    }
    if let Some(i) = s("Index Name") {
        label.push_str(&format!(" using {i}"));
    }
    if let Some(r) = s("Relation Name") {
        label.push_str(&format!(" on {r}"));
        if let Some(a) = s("Alias").filter(|a| *a != r) {
            label.push_str(&format!(" {a}"));
        }
    } else if let Some(c) = s("CTE Name") {
        label.push_str(&format!(" on {c}"));
    }
    let detail = ["Index Cond", "Hash Cond", "Merge Cond", "Join Filter", "Filter", "Sort Key", "Group Key"]
        .iter()
        .find_map(|k| {
            n.get(*k).map(|v| match v {
                Value::Array(a) => format!("{}: {}", k.to_lowercase(), a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ")),
                other => format!("{}: {}", k.to_lowercase(), other.as_str().unwrap_or(&other.to_string())),
            })
        });
    let loops = f("Actual Loops").unwrap_or(1.0);
    let actual_ms = if analyze { f("Actual Total Time").map(|t| t * loops) } else { None };
    let total = if analyze { actual_ms.unwrap_or(0.0) } else { f("Total Cost").unwrap_or(0.0) };
    let rows = if analyze { f("Actual Rows").map(|r| r * loops) } else { f("Plan Rows") };
    let idx = out.len();
    out.push(PlanNode { depth, label, detail, self_cost: total, total_cost: total, rows, actual_ms, last_child: last });
    let children = n.get("Plans").and_then(|p| p.as_array()).cloned().unwrap_or_default();
    let mut child_total = 0.0;
    let count = children.len();
    for (i, c) in children.iter().enumerate() {
        let before = out.len();
        walk_pg(c, depth + 1, i + 1 == count, analyze, out);
        child_total += out[before].total_cost;
    }
    out[idx].self_cost = (total - child_total).max(0.0);
}

/// MySQL `EXPLAIN FORMAT=TREE` / `EXPLAIN ANALYZE` text.
pub fn parse_mysql_tree(text: &str, analyze: bool) -> Plan {
    let mut plan = Plan { analyze, ..Default::default() };
    for line in text.lines() {
        let Some(pos) = line.find("-> ") else { continue };
        let depth = pos / 4;
        let body = &line[pos + 3..];
        let (label, rest) = match body.find("  (") {
            Some(i) => (&body[..i], &body[i..]),
            None => (body, ""),
        };
        let num = |key: &str, src: &str| -> Option<f64> {
            let i = src.find(key)? + key.len();
            let tail = &src[i..];
            let end = tail.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == 'e' || c == '+' || c == '-')).unwrap_or(tail.len());
            tail[..end].parse().ok()
        };
        let cost = num("cost=", rest).unwrap_or(0.0);
        let actual = rest.find("actual time=").map(|i| &rest[i..]).and_then(|a| {
            let range = &a["actual time=".len()..];
            let end_t = range.split_whitespace().next()?;
            let t: f64 = end_t.split("..").last()?.parse().ok()?;
            let loops = num("loops=", a).unwrap_or(1.0);
            Some(t * loops)
        });
        let rows = if analyze { rest.find("actual").and_then(|i| num("rows=", &rest[i..])) } else { num("rows=", rest) };
        let total = if analyze { actual.unwrap_or(0.0) } else { cost };
        plan.nodes.push(PlanNode {
            depth,
            label: label.trim().to_string(),
            detail: None,
            self_cost: total,
            total_cost: total,
            rows,
            actual_ms: actual,
            last_child: true,
        });
    }
    // self cost = total - direct children totals; last_child = no later sibling
    let n = plan.nodes.len();
    for i in 0..n {
        let d = plan.nodes[i].depth;
        let mut child_sum = 0.0;
        let mut j = i + 1;
        while j < n && plan.nodes[j].depth > d {
            if plan.nodes[j].depth == d + 1 {
                child_sum += plan.nodes[j].total_cost;
            }
            j += 1;
        }
        plan.nodes[i].self_cost = (plan.nodes[i].total_cost - child_sum).max(0.0);
        plan.nodes[i].last_child = !(j < n && plan.nodes[j].depth == d);
    }
    plan
}

/// Tree prefix for each node, e.g. `│  ├─ `.
pub fn prefixes(nodes: &[PlanNode], ascii: bool) -> Vec<String> {
    let (pipe, tee, elbow) = if ascii { ("|  ", "+- ", "`- ") } else { ("│  ", "├─ ", "└─ ") };
    let mut open: Vec<bool> = Vec::new(); // whether depth level still has siblings to come
    let mut out = Vec::new();
    for n in nodes {
        open.truncate(n.depth);
        let mut p = String::new();
        for (i, o) in open.iter().enumerate() {
            if i == 0 {
                continue;
            }
            p.push_str(if *o { pipe } else { "   " });
        }
        if n.depth > 0 {
            p.push_str(if n.last_child { elbow } else { tee });
        }
        out.push(p);
        open.push(!n.last_child);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pg_plan() {
        let json = r#"[{"Plan": {"Node Type": "Hash Join", "Join Type": "Left", "Total Cost": 100.0, "Plan Rows": 10, "Hash Cond": "(a.id = b.a_id)",
            "Plans": [
              {"Node Type": "Seq Scan", "Relation Name": "a", "Alias": "a", "Total Cost": 30.0, "Plan Rows": 100},
              {"Node Type": "Hash", "Total Cost": 50.0, "Plan Rows": 5, "Plans": [
                 {"Node Type": "Index Scan", "Index Name": "b_pkey", "Relation Name": "b", "Alias": "bb", "Total Cost": 45.0, "Plan Rows": 5}]}
            ]}, "Planning Time": 0.1}]"#;
        let p = parse_pg_json(json, false).unwrap();
        assert_eq!(p.nodes.len(), 4);
        assert_eq!(p.nodes[0].label, "Hash Left Join");
        assert_eq!(p.nodes[0].self_cost, 20.0);
        assert_eq!(p.nodes[3].label, "Index Scan using b_pkey on b bb");
        assert_eq!(p.nodes[2].self_cost, 5.0);
        assert_eq!(p.footer, vec!["planning 0.100 ms"]);
        let pre = prefixes(&p.nodes, false);
        assert_eq!(pre, vec!["", "├─ ", "└─ ", "   └─ "]);
    }

    #[test]
    fn mysql_tree() {
        let t = "-> Limit: 10 row(s)  (cost=5.5 rows=10)\n    -> Table scan on orders  (cost=4.25 rows=40)\n";
        let p = parse_mysql_tree(t, false);
        assert_eq!(p.nodes.len(), 2);
        assert_eq!(p.nodes[1].label, "Table scan on orders");
        assert!((p.nodes[0].self_cost - 1.25).abs() < 1e-9);
        let t = "-> Table scan on t  (cost=0.35 rows=1) (actual time=0.02..0.05 rows=3 loops=2)\n";
        let p = parse_mysql_tree(t, true);
        assert_eq!(p.nodes[0].actual_ms, Some(0.1));
        assert_eq!(p.nodes[0].rows, Some(3.0));
    }
}
