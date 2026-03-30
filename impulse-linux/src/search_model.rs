// SPDX-License-Identifier: GPL-3.0-only
//
// Search functionality bridge QObject for QML. Provides file name and
// content search via impulse_core::search.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, query)]
        #[qproperty(QString, root_path)]
        #[qproperty(QString, results_json)]
        #[qproperty(bool, is_searching)]
        #[qproperty(bool, case_sensitive)]
        #[qproperty(i32, result_count)]
        #[qproperty(QString, search_mode)]
        type SearchModel = super::SearchModelRust;

        #[qinvokable]
        fn search(self: Pin<&mut SearchModel>);

        #[qinvokable]
        fn search_files(self: Pin<&mut SearchModel>, query: &QString);

        #[qinvokable]
        fn search_content(self: Pin<&mut SearchModel>, query: &QString);

        #[qinvokable]
        fn clear(self: Pin<&mut SearchModel>);

        #[qsignal]
        fn search_completed(self: Pin<&mut SearchModel>);

        #[qsignal]
        fn result_selected(self: Pin<&mut SearchModel>, path: QString, line: i32);
    }
}

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Maximum number of search results to return.
const SEARCH_LIMIT: usize = 500;

pub struct SearchModelRust {
    query: QString,
    root_path: QString,
    results_json: QString,
    is_searching: bool,
    case_sensitive: bool,
    result_count: i32,
    search_mode: QString,
    /// Cancellation flag for the current search.
    cancel_flag: Arc<AtomicBool>,
}

impl Default for SearchModelRust {
    fn default() -> Self {
        Self {
            query: QString::default(),
            root_path: QString::default(),
            results_json: QString::from("[]"),
            is_searching: false,
            case_sensitive: false,
            result_count: 0,
            search_mode: QString::from("content"),
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl qobject::SearchModel {
    pub fn search(mut self: Pin<&mut Self>) {
        let query_str = self.as_ref().query().to_string();
        let root_str = self.as_ref().root_path().to_string();
        let mode_str = self.as_ref().search_mode().to_string();

        if query_str.is_empty() || root_str.is_empty() {
            self.as_mut().set_results_json(QString::from("[]"));
            self.as_mut().set_result_count(0);
            self.as_mut().search_completed();
            return;
        }

        // Cancel any ongoing search
        self.as_ref().rust().cancel_flag.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.as_mut().rust_mut().cancel_flag = cancel.clone();

        self.as_mut().set_is_searching(true);

        let case_sensitive = *self.as_ref().case_sensitive();

        let search_type = match mode_str.as_str() {
            "files" => "filename",
            "content" => "content",
            "both" => "both",
            _ => "content",
        };

        let result = impulse_core::search::search(
            &root_str,
            &query_str,
            search_type,
            case_sensitive,
            SEARCH_LIMIT,
            Some(&cancel),
        );

        self.as_mut().set_is_searching(false);

        match result {
            Ok(results) => {
                let count = results.len() as i32;
                let json =
                    serde_json::to_string(&results).unwrap_or_else(|_| "[]".to_string());
                self.as_mut().set_results_json(QString::from(json.as_str()));
                self.as_mut().set_result_count(count);
            }
            Err(e) => {
                log::warn!("Search failed: {}", e);
                self.as_mut().set_results_json(QString::from("[]"));
                self.as_mut().set_result_count(0);
            }
        }

        self.as_mut().search_completed();
    }

    pub fn search_files(mut self: Pin<&mut Self>, query: &QString) {
        self.as_mut().set_query(query.clone());
        self.as_mut().set_search_mode(QString::from("files"));
        self.as_mut().search();
    }

    pub fn search_content(mut self: Pin<&mut Self>, query: &QString) {
        self.as_mut().set_query(query.clone());
        self.as_mut().set_search_mode(QString::from("content"));
        self.as_mut().search();
    }

    pub fn clear(mut self: Pin<&mut Self>) {
        // Cancel any ongoing search
        self.as_ref().rust().cancel_flag.store(true, Ordering::Relaxed);

        self.as_mut().set_query(QString::default());
        self.as_mut().set_results_json(QString::from("[]"));
        self.as_mut().set_result_count(0);
        self.as_mut().set_is_searching(false);
    }
}
