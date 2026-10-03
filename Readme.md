# Project Hover Built on Squads Smart Account Program v0.1 

## This Project Contains Four Components/Respositoires 

1. Backend : https://github.com/YATHARTH-Sriv/althoverbackend
2. Node Backend Service For Agent and Uploads : https://github.com/YATHARTH-Sriv/hovernode
3. Frontend : https://github.com/YATHARTH-Sriv/hoverfront 
4. Hover Extension : https://github.com/YATHARTH-Sriv/hoverextension

**Note: No IDL was used so thier is no coralxyzanchor used anywhere** 

## In this We Talk About The Rust ( Aum ) Backend 

## Architecure : 

    Frontend :3000
        |
        +--> Rust API :9000
        |      |
        |      +--> PostgreSQL / Neon
        |      +--> Solana RPC / Surfpool
        |      +--> Node internal endpoints
        |
        +--> Node API :8000
            |
            +--> AI SDK and model provider
            +--> Vercel Blob
            +--> OCR processing
            +--> Rust internal API calls

## Why we Have A Different Node Service :

I started writing the backend in node express after completing it I did rewrite the routes handlers and solana setup to rust but left out the the ocr agent bill uploads and AI SDK part to node itself because we are using vercel blob which works great with typescript and also ai sdk by vercel is used which is much more rich in node 

## Sources Of Truth 

1. Solana: Smart accounts, settings, signers, proposals, approvals, rejections, execution, and transferred funds.
2. PostgreSQL: profiles, invoices, receipts, invitations, payout context, and indexed blockchain data.
3. Node service: AI and file-processing capabilities.
4. Rust service: Main business API and orchestration layer.

## Folder Structure For All The Services : 

### Settings Management

    settings_management/
    ├── mod.rs
    ├── models.rs
    ├── handlers.rs
    ├── service.rs
    └── repository.rs

- `models.rs`: Request structs, response structs, and SQLx database row types used by settings management.

- `handlers.rs`: HTTP handlers for changing the threshold, adding signers, removing signers, and refreshing settings.

- `service.rs`: Solana-related settings logic such as fetching the on-chain account, checking the authority, building unsigned transactions, and verifying submitted transactions.

- `repository.rs`: PostgreSQL queries for settings, signer synchronization, activity logs, user profiles, workspace membership, and signer invitations.

### Transaction Lifecycle

    transaction_lifecycle/
    ├── mod.rs
    ├── models.rs
    ├── handlers.rs
    ├── service.rs
    └── repository.rs

- `models.rs`: Models for transaction creation, proposals, approvals, rejections, execution, payout context, and API responses.

- `handlers.rs`: Builds and verifies the complete Squads transaction lifecycle: create transaction, create proposal, approve, reject, execute, and refresh.

- `service.rs`: Reads proposal accounts from Solana, synchronizes proposal status and votes, detects stale proposals, and prepares transaction responses.

- `repository.rs`: Loads settings, checks signer permissions, loads transactions, approvals, and payout context from PostgreSQL.

### Invoices

    invoices/
    ├── mod.rs
    ├── models.rs
    ├── handlers.rs
    ├── service.rs
    └── repository.rs

- `models.rs`: Invoice, line-item, payment-link, settlement, request, and response models.

- `handlers.rs`: HTTP endpoints for creating drafts, updating invoices, sending invoices, opening public payment links, and confirming payments.

- `service.rs`: Invoice totals, public token handling, payment verification, and settlement-related business logic.

- `repository.rs`: SQL queries for invoices, customers, line items, payment links, and settlement records.

### Receipts

    receipts/
    ├── mod.rs
    ├── models.rs
    ├── handlers.rs
    └── processor.rs

- `models.rs`: Receipt request, response, extraction, and database models.

- `handlers.rs`: Endpoints for scanning, updating, deleting, viewing, and paying receipts.

- `processor.rs`: Communicates with the Node internal service for file storage and extraction, then stores the processed receipt in PostgreSQL.

## Solana Setup

The `src/solanasetup` folder contains lower-level Solana utilities.

    solanasetup/
    ├── mod.rs
    ├── config.rs
    ├── constant.rs
    ├── helpers.rs
    ├── pda.rs
    └── transactions.rs

- `config.rs`: Creates the non-blocking Solana RPC client.

- `constant.rs`: Stores the Squads program ID, System Program ID, and PDA seed constants.

- `helpers.rs`: Creates Anchor instruction and account discriminators using SHA-256.

- `pda.rs`: Derives settings, smart-account, transaction, proposal, and program-configuration PDAs.

- `transactions.rs`: Builds unsigned Base64 transactions, decodes signed transactions, submits them, and verifies confirmed Solana and Squads instructions.

## Squads Account Definitions

The `src/squadsaccounts` folder contains the manual Squads program interface used instead of a generated Anchor client.

    squadsaccounts/
    ├── mod.rs
    ├── programconfig.rs
    ├── settingsconfig.rs
    └── transactionconfig.rs

- `programconfig.rs`: Defines and decodes the Squads global program-configuration account.

- `settingsconfig.rs`: Defines the settings-account layout, signer permissions, and instructions for creating settings, changing thresholds, and adding or removing signers.

- `transactionconfig.rs`: Defines proposal and transaction layouts and builds instructions for creating, approving, rejecting, and executing Squads transactions.

These files manually handle Anchor discriminators, Borsh serialization, PDA derivation, and account ordering. This is why the backend can communicate with the Squads program without using its IDL.

**Work Left is To Do Tests Coverage**
---
