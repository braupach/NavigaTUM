use crate::github;
use crate::proposed_edits::tmp_repo::TempRepo;
use crate::tokens::RecordedTokens;
use actix_web::web::{Data, Json};
use actix_web::{post, HttpResponse};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

mod discription;
mod tmp_repo;

#[derive(Deserialize, Clone)]
struct Edit {}
pub trait AppliableEdit {
    fn apply(&self, key: &str, base_dir: &Path) -> String;
}

#[derive(Deserialize)]
pub struct EditRequest {
    token: String,
    edits: HashMap<String, Edit>,
    additional_context: String,
    privacy_checked: bool,
}

const GIT_URL: &str = "git@github.com:TUM-Dev/NavigaTUM.git";
impl EditRequest {
    async fn apply_changes_and_generate_description(
        &self,
        branch_name: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let repo = TempRepo::clone_and_checkout(GIT_URL, branch_name).await?;
        let desc = repo.apply_and_gen_description(self)?;
        repo.commit(&desc.title).await?;
        repo.push().await?;
        Ok(desc.body)
    }
    fn edits_for<T: AppliableEdit>(&self, extractor: fn(Edit) -> Option<T>) -> HashMap<String, T> {
        self.edits
            .clone()
            .into_iter()
            .filter_map(|(k, edit)| extractor(edit).map(|coord| (k, coord)))
            .collect()
    }

    fn extract_labels(&self) -> Vec<String> {
        vec!["webform".to_string()]
    }
    fn extract_subject(&self) -> String {
        "No Edits".to_string()
    }
}

#[post("/api/feedback/propose_edit")]
pub async fn propose_edits(
    recorded_tokens: Data<RecordedTokens>,
    req_data: Json<EditRequest>,
) -> HttpResponse {
    // auth
    if let Some(e) = recorded_tokens.validate(&req_data.token).await {
        return e;
    }

    // validate request
    if !req_data.privacy_checked {
        return HttpResponse::UnavailableForLegalReasons()
            .content_type("text/plain")
            .body("Using this endpoint without accepting the privacy policy is not allowed");
    };
    if !req_data.edits.is_empty() {
        return HttpResponse::UnprocessableEntity()
            .content_type("text/plain")
            .body("Not enough edits provided");
    };
    if req_data.edits.len() > 500 {
        return HttpResponse::InsufficientStorage()
            .content_type("text/plain")
            .body("Too many edits provided");
    };

    let branch_name = format!("usergenerated/request-{}", rand::random::<u16>());
    match req_data
        .apply_changes_and_generate_description(&branch_name)
        .await
    {
        Ok(description) => {
            github::open_pr(
                branch_name,
                &format!(
                    "[User-Provided] {subject}",
                    subject = req_data.extract_subject()
                ),
                &description,
                req_data.extract_labels(),
            )
            .await
        }
        Err(e) => {
            log::error!("Error while applying changes: {e}", e = e);
            HttpResponse::InternalServerError().finish()
        }
    }
}