# Makefile for D9 Contract Management

# Colors for output
GREEN := \033[0;32m
RED := \033[0;31m
YELLOW := \033[0;33m
BLUE := \033[0;34m
NC := \033[0m # No Color

# Contract directories
CONTRACTS := market-maker merchant-mining mining-pool
CONTRACT_DIR := ./

# Upload history file
UPLOAD_HISTORY := upload-history.json

# Default target
.DEFAULT_GOAL := help

# Help target
.PHONY: help
help:
	@echo "$(BLUE)D9 Contract Management$(NC)"
	@echo "$(BLUE)=====================$(NC)"
	@echo ""
	@echo "$(GREEN)Available commands:$(NC)"
	@echo ""
	@echo "$(YELLOW)Checking & Building:$(NC)"
	@echo "  make check-all                    - Check all contracts"
	@echo "  make check-contract CONTRACT=name - Check specific contract"
	@echo "  make build-contract CONTRACT=name - Build specific contract"
	@echo ""
	@echo "$(YELLOW)Upload:$(NC)"
	@echo "  make upload-local CONTRACT=name   - Upload to local network (uses //Alice)"
	@echo "  make upload-testnet CONTRACT=name SURI=key - Upload to testnet"
	@echo "  make upload-mainnet CONTRACT=name SURI=key - Upload to mainnet (requires approval)"
	@echo "  make upload-code CONTRACT=name NETWORK=net SURI=key - Upload code only"
	@echo ""
	@echo "$(YELLOW)History:$(NC)"
	@echo "  make history                      - Show all upload history"
	@echo "  make history-contract CONTRACT=name - Show history for specific contract"
	@echo "  make history-latest               - Show latest uploads only"
	@echo ""
	@echo "$(YELLOW)Examples:$(NC)"
	@echo "  make check-all"
	@echo "  make upload-local CONTRACT=mining-pool"
	@echo "  make upload-testnet CONTRACT=mining-pool SURI=\"//Alice\""
	@echo ""

# Check all contracts
.PHONY: check-all
check-all:
	@echo "$(GREEN)Checking all contracts...$(NC)"
	@for contract in $(CONTRACTS); do \
		echo "$(YELLOW)Checking $$contract...$(NC)"; \
		$(MAKE) check-contract CONTRACT=$$contract || exit 1; \
	done
	@echo "$(GREEN)All contracts passed!$(NC)"

# Check individual contract
.PHONY: check-contract
check-contract:
	@echo "$(YELLOW)Running checks for $(CONTRACT)...$(NC)"
	@cd $(CONTRACT) && cargo check --all-features
	@cd $(CONTRACT) && cargo test
	@$(MAKE) verify-storage CONTRACT=$(CONTRACT)

# Verify storage hasn't changed
.PHONY: verify-storage
verify-storage:
	@echo "$(YELLOW)Verifying storage for $(CONTRACT)...$(NC)"
	@python3 scripts/verify_storage.py check $(CONTRACT)

# Build contract with metadata
.PHONY: build-contract
build-contract:
	@echo "$(YELLOW)Building $(CONTRACT)...$(NC)"
	@cd $(CONTRACT) && cargo contract build --release
	@echo "$(GREEN)Build successful!$(NC)"

# Pre-upload check
.PHONY: pre-upload
pre-upload:
	@echo "$(YELLOW)Running pre-upload checks for $(CONTRACT)...$(NC)"
	@$(MAKE) check-contract CONTRACT=$(CONTRACT)
	@$(MAKE) build-contract CONTRACT=$(CONTRACT)
	@python3 scripts/compare_metadata.py $(CONTRACT)
	@echo "$(GREEN)Pre-upload checks passed!$(NC)"

# Upload contract with history tracking
.PHONY: upload
upload:
	@if [ -z "$(CONTRACT)" ]; then \
		echo "$(RED)Error: CONTRACT not specified$(NC)"; \
		echo "Usage: make upload CONTRACT=<contract-name> NETWORK=<network> SURI=<secret-uri>"; \
		exit 1; \
	fi
	@if [ -z "$(NETWORK)" ]; then \
		echo "$(RED)Error: NETWORK not specified$(NC)"; \
		echo "Usage: make upload CONTRACT=<contract-name> NETWORK=<network> SURI=<secret-uri>"; \
		exit 1; \
	fi
	@if [ -z "$(SURI)" ]; then \
		echo "$(RED)Error: SURI not specified$(NC)"; \
		echo "Usage: make upload CONTRACT=<contract-name> NETWORK=<network> SURI=<secret-uri>"; \
		exit 1; \
	fi
	@echo "$(YELLOW)Uploading $(CONTRACT) to $(NETWORK)...$(NC)"
	@python3 scripts/upload_code.py $(CONTRACT) $(NETWORK) --suri "$(SURI)" $(UPLOAD_ARGS)

# Upload only code (upload without instantiation)
.PHONY: upload-code
upload-code:
	@$(MAKE) upload UPLOAD_ARGS="--upload-only"

# Upload to local network (convenience target)
.PHONY: upload-local
upload-local:
	@if [ -z "$(CONTRACT)" ]; then \
		echo "$(RED)Error: CONTRACT not specified$(NC)"; \
		echo "Usage: make upload-local CONTRACT=<contract-name>"; \
		exit 1; \
	fi
	@$(MAKE) upload CONTRACT=$(CONTRACT) NETWORK=local SURI="//Alice"

# Upload to testnet
.PHONY: upload-testnet
upload-testnet:
	@if [ -z "$(CONTRACT)" ]; then \
		echo "$(RED)Error: CONTRACT not specified$(NC)"; \
		echo "Usage: make upload-testnet CONTRACT=<contract-name> SURI=<secret-uri>"; \
		exit 1; \
	fi
	@if [ -z "$(SURI)" ]; then \
		echo "$(RED)Error: SURI not specified$(NC)"; \
		echo "Usage: make upload-testnet CONTRACT=<contract-name> SURI=<secret-uri>"; \
		exit 1; \
	fi
	@$(MAKE) upload CONTRACT=$(CONTRACT) NETWORK=testnet SURI="$(SURI)"

# Upload to mainnet (with extra confirmation)
.PHONY: upload-mainnet
upload-mainnet:
	@if [ -z "$(CONTRACT)" ]; then \
		echo "$(RED)Error: CONTRACT not specified$(NC)"; \
		echo "Usage: make upload-mainnet CONTRACT=<contract-name> SURI=<secret-uri>"; \
		exit 1; \
	fi
	@if [ -z "$(SURI)" ]; then \
		echo "$(RED)Error: SURI not specified$(NC)"; \
		echo "Usage: make upload-mainnet CONTRACT=<contract-name> SURI=<secret-uri>"; \
		exit 1; \
	fi
	@echo "$(RED)⚠️  WARNING: This will upload to MAINNET! ⚠️$(NC)"
	@$(MAKE) upload CONTRACT=$(CONTRACT) NETWORK=mainnet SURI="$(SURI)"

# Show upload history
.PHONY: history
history:
	@python3 scripts/show_history.py $(UPLOAD_HISTORY)

# Show history for specific contract
.PHONY: history-contract
history-contract:
	@if [ -z "$(CONTRACT)" ]; then \
		echo "$(RED)Error: CONTRACT not specified$(NC)"; \
		echo "Usage: make history-contract CONTRACT=<contract-name>"; \
		exit 1; \
	fi
	@python3 scripts/show_history.py $(UPLOAD_HISTORY) --contract $(CONTRACT)

# Show latest uploads
.PHONY: history-latest
history-latest:
	@python3 scripts/show_history.py $(UPLOAD_HISTORY) --latest