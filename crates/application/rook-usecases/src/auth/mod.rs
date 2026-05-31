// auth — authentication use cases for MANAGEMENT routes
//
// Exposes EnsureAdminUser, SetAdminPassword, Login, Logout, and ValidateSession for auth flows.

pub mod bootstrap_status;
pub mod ensure_admin_user;
pub mod login;
pub mod logout;
pub mod set_admin_password;
pub mod validate_session;

pub use bootstrap_status::{
    BootstrapSetupError, BootstrapSetupInput, BootstrapSetupOutput, BootstrapState,
    BootstrapStatus, BootstrapStatusError,
};
pub use ensure_admin_user::{EnsureAdminUser, EnsureAdminUserError};
pub use login::{Login, LoginError, LoginInput, LoginOutput};
pub use logout::{Logout, LogoutError, LogoutInput};
pub use set_admin_password::{SetAdminPassword, SetAdminPasswordError, SetAdminPasswordInput};
pub use validate_session::{ValidateSession, ValidateSessionError, ValidatedSession};
