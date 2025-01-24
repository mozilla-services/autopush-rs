use std::collections::HashMap;

use actix_web::{web::Data, HttpResponse};
use chrono;

use crate::server::AppState;

const METRIC_NAME: &str = "autopush_reliability";

/// Generate a Prometheus compatible report. Output should follow the
/// [instrumentation](https://prometheus.io/docs/practices/instrumentation/) guidelines.
///
/// In short form, the file should be a plain text output, with each metric on it's own line
/// using the following format:
/// ```text
/// # HELP metric_name Optional description of this metric
/// # TYPE metric_name {required type (gauge|count|histogram|summary)}
/// metric_name{label="label1"} value
/// metric_name{label="label2"} value
/// ```
/// An example which would return counts of messages in given states at the current
/// time would be:
/// ```text
/// # HELP autopush_reliability Counts for messages in given states
/// # TYPE metric_name gauge
/// autopush_reliability{label="recv"} 123
/// autopush_reliability{label="stor"} 123
/// ///
/// ```
/// Note that time is not required. A timestamp has been added to the output, but is
/// ignored by Prometheus, and is only provided to ensure that there is no intermediate
/// caching occurring.
///
pub fn gen_report(report: Option<HashMap<String, i32>>) -> String {
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S.%f");
    let mut buffer: Vec<String> = Vec::new();
    buffer.push(format!("# Created {timestamp}"));
    match report {
        Some(v) => {
            debug!("🔍 Reporting {:?}", &v);
            buffer.push(format!(
                "# HELP {METRIC_NAME} Message counts for given states"
            ));
            buffer.push(format!("# TYPE {METRIC_NAME} gauge"));
            for (milestone, value) in v.into_iter() {
                buffer.push(format!("{METRIC_NAME}{{label=\"{milestone}\"}} {value}"))
            }
            buffer.join("\n")
        }
        None => {
            debug!("🔍 Reporting, but nothing to report");
            buffer.push("# No data found".to_owned());
            buffer.join("\n")
        }
    }
}

pub async fn report_handler(app_state: Data<AppState>) -> HttpResponse {
    let reliability = app_state.reliability.clone();
    if let Err(err) = reliability.gc().await {
        error!("🔍🟥 Reporting, Error {:?}", &err);
        return HttpResponse::InternalServerError()
            .content_type("text/plain")
            .body(format!("# ERROR: {err}\n"));
    };
    match reliability.report().await {
        Ok(report) => HttpResponse::Ok()
            .content_type("text/plain")
            .body(gen_report(report)),
        Err(e) => {
            error!("🔍🟥 Reporting, Error {:?}", &e);
            HttpResponse::InternalServerError()
                .content_type("text/plain")
                .body(format!("# ERROR: {e}\n"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use autopush_common::reliability::ReliabilityState;

    #[test]
    fn test_report() {
        // create a nonce report
        let mut report: HashMap<String, i32> = HashMap::new();
        let acpt = ReliabilityState::Accepted.to_string();
        let trns = ReliabilityState::Transmitted.to_string();
        report.insert(acpt.clone(), 111);
        report.insert(ReliabilityState::Stored.to_string(), 222);
        report.insert(ReliabilityState::Retrieved.to_string(), 333);
        report.insert(trns.clone(), 444);

        let generated = gen_report(Some(report));
        // We don't really care if the `Created` or `HELP` lines are included
        assert!(generated.contains(&format!("# TYPE {METRIC_NAME}")));
        // sample the first and last values.
        assert!(generated.contains(&format!("{METRIC_NAME}{{label=\"{acpt}\"}} 111")));
        assert!(generated.contains(&format!("{METRIC_NAME}{{label=\"{trns}\"}} 444")));
    }
}
