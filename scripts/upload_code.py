#!/usr/bin/env python3
"""
Deployment script using cargo contract
"""

import os
import sys
import json
import subprocess
import hashlib
from datetime import datetime
from typing import Dict, Optional, Tuple

class ContractDeployer:
    def __init__(self):
        self.history_file = "upload-history.json"
        self.approvals_file = ".github/pr-approvals.json"
        self.networks = {
            "local": {
                "url": "ws://localhost:9944",
                "allowed_branches": None,  # Any branch
                "requires_approval": False
            },
            "testnet": {
                "url": "wss://testnet.d9network.org:4030",
                "allowed_branches": ["main", "develop", "feature/*"],
                "requires_approval": False
            },
            "mainnet": {
                "url": "wss://mainnet.d9network.com:40300", 
                "allowed_branches": ["main"],
                "requires_approval": True
            }
        }
    
    def get_current_branch(self) -> str:
        """Get current git branch"""
        result = subprocess.run(
            ["git", "rev-parse", "--abbrev-ref", "HEAD"],
            capture_output=True,
            text=True
        )
        return result.stdout.strip()
    
    def calculate_code_hash(self, wasm_path: str) -> str:
        """Calculate blake2b hash of wasm file"""
        with open(wasm_path, 'rb') as f:
            return hashlib.blake2b(f.read(), digest_size=32).hexdigest()
    
    def get_approved_hash(self, contract: str) -> Optional[str]:
        """Get approved hash from PR approvals"""
        if not os.path.exists(self.approvals_file):
            return None
            
        with open(self.approvals_file, 'r') as f:
            approvals = json.load(f)
            approval = approvals.get(contract)
            if approval:
                return approval.get("code_hash")
        return None
    
    def verify_deployment_allowed(self, contract: str, network: str) -> Tuple[bool, str]:
        """Verify if deployment is allowed"""
        network_config = self.networks[network]
        current_branch = self.get_current_branch()
        
        # Check branch restrictions
        allowed_branches = network_config["allowed_branches"]
        if allowed_branches:
            branch_ok = False
            for pattern in allowed_branches:
                if pattern.endswith("/*"):
                    if current_branch.startswith(pattern[:-2]):
                        branch_ok = True
                        break
                elif current_branch == pattern:
                    branch_ok = True
                    break
            
            if not branch_ok:
                return False, f"Branch '{current_branch}' not allowed for {network}"
        
        # Check hash approval for mainnet
        if network_config["requires_approval"]:
            wasm_path = f"{contract}/target/ink/{contract}.wasm"
            if not os.path.exists(wasm_path):
                return False, "Contract not built"
                
            current_hash = self.calculate_code_hash(wasm_path)
            approved_hash = self.get_approved_hash(contract)
            
            if not approved_hash:
                return False, "No approved hash found for mainnet deployment"
                
            if current_hash != approved_hash:
                return False, f"Hash mismatch. Approved: {approved_hash}, Current: {current_hash}"
        
        return True, "OK"
    
    def build_contract(self, contract: str) -> bool:
        """Build contract if needed"""
        print(f"🔨 Building {contract}...")
        result = subprocess.run(
            ["cargo", "contract", "build", "--release"],
            cwd=contract,
            capture_output=True,
            text=True
        )
        
        if result.returncode != 0:
            print(f"❌ Build failed:\n{result.stderr}")
            return False
            
        print("✅ Build successful")
        return True
    
    def upload_contract(self, contract: str, network: str, suri: str) -> Optional[str]:
        """Upload contract code and return code hash"""
        print(f"\n📤 Uploading contract code to {network}...")
        
        wasm_path = f"{contract}/target/ink/{contract}.wasm"
        network_url = self.networks[network]["url"]
        
        # Use cargo contract upload
        cmd = [
            "cargo", "contract", "upload",
            "--manifest-path", f"{contract}/Cargo.toml",
            "--url", network_url,
            "--suri", suri,
            "--skip-confirm"
        ]
        
        result = subprocess.run(cmd, capture_output=True, text=True)
        
        if result.returncode != 0:
            print(f"❌ Upload failed:\n{result.stderr}")
            return None
        
        # Parse output to get code hash
        # cargo contract outputs: "Code hash 0x..."
        for line in result.stdout.split('\n'):
            if "Code hash" in line:
                code_hash = line.split("Code hash")[1].strip()
                print(f"✅ Code uploaded with hash: {code_hash}")
                return code_hash
        
        return None
    
    
    def deploy(self, contract: str, network: str, suri: str, 
               upload_only: bool = False, constructor: str = "new", args: list = None):
        """Main deployment function"""
        print(f"\n🚀 Deploying {contract} to {network}")
        print("=" * 60)
        
        # Verify deployment is allowed
        allowed, message = self.verify_deployment_allowed(contract, network)
        if not allowed:
            print(f"❌ Deployment not allowed: {message}")
            sys.exit(1)
        
        print(f"✅ Deployment checks passed: {message}")
        
        # Build contract
        if not self.build_contract(contract):
            sys.exit(1)
        
        # Get code hash for recording
        wasm_path = f"{contract}/target/ink/{contract}.wasm"
        local_code_hash = self.calculate_code_hash(wasm_path)
        
        # Confirm deployment
        print(f"\n📋 Deployment Summary:")
        print(f"   Contract: {contract}")
        print(f"   Network: {network}")
        print(f"   Branch: {self.get_current_branch()}")
        print(f"   Code Hash: {local_code_hash}")
        
        if network == "mainnet":
            print("\n🚨 MAINNET DEPLOYMENT WARNING 🚨")
            confirm = input("Type 'DEPLOY TO MAINNET' to continue: ")
            if confirm != "DEPLOY TO MAINNET":
                print("Deployment cancelled")
                return
        else:
            confirm = input("\nContinue? (y/n): ")
            if confirm.lower() != 'y':
                print("Deployment cancelled")
                return
        
        # Upload code
        code_hash = self.upload_contract(contract, network, suri)
        if not code_hash:
            sys.exit(1)
        
        contract_address = None
        if not upload_only:
            print("❌ Contract instantiation not supported. Use --upload-only flag.")
            sys.exit(1)
        
        # Record deployment
        self.record_deployment(
            contract=contract,
            network=network,
            code_hash=code_hash,
            contract_address=contract_address,
            upload_only=upload_only
        )
        
        print(f"\n✅ Deployment completed successfully!")
    
    def record_deployment(self, contract: str, network: str, code_hash: str, 
                         contract_address: Optional[str], upload_only: bool):
        """Record deployment in history"""
        history = {}
        if os.path.exists(self.history_file):
            with open(self.history_file, 'r') as f:
                history = json.load(f)
        
        if contract not in history:
            history[contract] = []
        
        deployment = {
            "timestamp": datetime.now().isoformat(),
            "network": network,
            "branch": self.get_current_branch(),
            "code_hash": code_hash,
            "contract_address": contract_address,
            "upload_only": upload_only,
            "deployed_by": os.environ.get("USER", "unknown"),
            "git_commit": subprocess.run(
                ["git", "rev-parse", "HEAD"],
                capture_output=True,
                text=True
            ).stdout.strip()
        }
        
        # Get previous deployment if exists
        if history[contract]:
            deployment["previous_code_hash"] = history[contract][-1].get("code_hash")
        
        history[contract].append(deployment)
        
        with open(self.history_file, 'w') as f:
            json.dump(history, f, indent=2)


def main():
    import argparse
    
    parser = argparse.ArgumentParser(description="Deploy ink! contracts")
    parser.add_argument("contract", help="Contract name (e.g., mining-pool)")
    parser.add_argument("network", choices=["local", "testnet", "mainnet"])
    parser.add_argument("--suri", required=True, help="Secret URI for signing")
    parser.add_argument("--upload-only", action="store_true", 
                       help="Only upload code, don't instantiate")
    parser.add_argument("--constructor", default="new", 
                       help="Constructor function name")
    parser.add_argument("--args", nargs="*", 
                       help="Constructor arguments")
    
    args = parser.parse_args()
    
    deployer = ContractDeployer()
    deployer.deploy(
        contract=args.contract,
        network=args.network,
        suri=args.suri,
        upload_only=args.upload_only,
        constructor=args.constructor,
        args=args.args
    )


if __name__ == "__main__":
    main()