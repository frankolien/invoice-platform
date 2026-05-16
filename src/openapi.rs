use utoipa::{
    Modify, OpenApi,
    openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
};

use crate::modules::{
    analytics, auth, client, invoice, organization, payment, recurring_invoice,
    webhook_subscription,
};

pub struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi
            .components
            .as_mut()
            .expect("components are populated by the derive");
        components.add_security_scheme(
            "bearer_auth",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
    }
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Invoice Platform API",
        version = "0.1.0",
        description = "Multi-tenant invoicing, payments (Stripe), recurring billing, and outbound webhooks.",
        license(name = "MIT")
    ),
    servers(
        (url = "/", description = "Current host")
    ),
    paths(
        auth::register,
        auth::login,
        auth::refresh_tokens,
        organization::create,
        organization::get_one,
        organization::update,
        organization::invite,
        client::create,
        client::list,
        client::get_one,
        client::update,
        client::soft_delete,
        invoice::create,
        invoice::list,
        invoice::get_one,
        invoice::update,
        invoice::send,
        invoice::cancel,
        invoice::mark_viewed,
        payment::create_checkout,
        payment::list_payments_for_invoice,
        payment::refund,
        payment::webhook::stripe_webhook,
        recurring_invoice::create,
        recurring_invoice::list,
        recurring_invoice::get_one,
        recurring_invoice::update,
        recurring_invoice::pause,
        recurring_invoice::resume,
        recurring_invoice::cancel,
        webhook_subscription::create,
        webhook_subscription::list,
        webhook_subscription::get_one,
        webhook_subscription::update,
        webhook_subscription::delete_one,
        webhook_subscription::test_delivery,
        analytics::revenue_report,
        analytics::invoice_report,
    ),
    components(schemas(
        auth::RegisterInput,
        auth::LoginInput,
        auth::RefreshInput,
        auth::AuthResponse,
        auth::UserDto,
        auth::TokenPair,
        organization::CreateOrgInput,
        organization::UpdateOrgInput,
        organization::InviteInput,
        organization::Organization,
        client::CreateClientInput,
        client::UpdateClientInput,
        client::Client,
        client::ClientPage,
        invoice::LineItem,
        invoice::CreateInvoiceInput,
        invoice::UpdateInvoiceInput,
        invoice::Invoice,
        payment::Payment,
        payment::RefundInput,
        payment::CheckoutSessionResponse,
        payment::RefundResponse,
        recurring_invoice::CreateInput,
        recurring_invoice::UpdateInput,
        recurring_invoice::RecurringInvoice,
        webhook_subscription::CreateInput,
        webhook_subscription::UpdateInput,
        webhook_subscription::Subscription,
        analytics::RevenueReport,
        analytics::MonthlyRevenue,
        analytics::InvoiceReport,
        analytics::StatusGroup,
        analytics::OverdueSummary,
    )),
    modifiers(&SecurityAddon),
    tags(
        (name = "auth", description = "Authentication and token refresh"),
        (name = "organizations", description = "Organization & membership management"),
        (name = "clients", description = "Tenant clients (customers being invoiced)"),
        (name = "invoices", description = "Invoices: create, send, cancel, view"),
        (name = "payments", description = "Stripe Checkout sessions and refunds"),
        (name = "recurring-invoices", description = "Recurring invoice schedules"),
        (name = "webhook-subscriptions", description = "Outbound webhook subscriptions"),
        (name = "webhooks", description = "Inbound webhooks (Stripe)"),
        (name = "analytics", description = "Revenue and invoice reporting"),
    )
)]
pub struct ApiDoc;
