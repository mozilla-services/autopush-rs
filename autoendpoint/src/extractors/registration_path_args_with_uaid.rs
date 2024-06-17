use crate::error::{ApiError, ApiErrorKind};
use crate::extractors::registration_path_args::RegistrationPathArgs;
use crate::extractors::routers::RouterType;
use crate::server::AppState;
use actix_web::dev::Payload;
use actix_web::web::Data;
use actix_web::{FromRequest, HttpRequest};
use futures::future::LocalBoxFuture;
use futures::FutureExt;
use uuid::Uuid;

use autopush_common::db::User;

/// An extension of `RegistrationPathArgs` which requires a `uaid` path arg.
/// The `uaid` is verified by checking if the user exists in the database.
pub struct RegistrationPathArgsWithUaid {
    pub router_type: RouterType,
    pub app_id: String,
    pub user: User,
}

impl FromRequest for RegistrationPathArgsWithUaid {
    type Error = ApiError;
    type Future = LocalBoxFuture<'static, Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let req = req.clone();

        async move {
            let app_state: Data<AppState> = Data::extract(&req)
                .into_inner()
                .expect("No server state found");
            let path_args = RegistrationPathArgs::extract(&req).into_inner()?;
            let uaid = req
                .match_info()
                .get("uaid")
                .expect("{uaid} must be part of the path")
                .parse::<Uuid>()
                .map_err(|_| ApiErrorKind::NoUser)?;

            // Verify that the user exists
            let Some(user) = app_state.db.get_user(&uaid).await? else {
                return Err(ApiErrorKind::NoUser.into());
            };

            Ok(Self {
                router_type: path_args.router_type,
                app_id: path_args.app_id,
                user,
            })
        }
        .boxed_local()
    }
}
