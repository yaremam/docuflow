use axum::extract::Form;
use axum::http::StatusCode;

use crate::web::forms::Credentials;
use crate::web::templates::{ComingSoonTemplate, LoginTemplate, SignupTemplate};

fn not_yet_implemented(feature_name: &'static str) -> (StatusCode, ComingSoonTemplate) {
    (
        StatusCode::NOT_IMPLEMENTED,
        ComingSoonTemplate {
            feature_name,
            active_tab: "",
        },
    )
}

#[tracing::instrument]
pub async fn signup_form() -> SignupTemplate {
    SignupTemplate { active_tab: "signup" }
}

#[tracing::instrument(skip(form))]
pub async fn signup_submit(Form(form): Form<Credentials>) -> (StatusCode, ComingSoonTemplate) {
    let _ = form;
    not_yet_implemented("Signup")
}

#[tracing::instrument]
pub async fn login_form() -> LoginTemplate {
    LoginTemplate { active_tab: "login" }
}

#[tracing::instrument(skip(form))]
pub async fn login_submit(Form(form): Form<Credentials>) -> (StatusCode, ComingSoonTemplate) {
    let _ = form;
    not_yet_implemented("Login")
}
