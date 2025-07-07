#!/usr/bin/env python3
"""
Display deployment history for D9 contracts
"""

import json
import sys
import os
from datetime import datetime
from typing import Dict, List, Optional

class DeploymentHistoryViewer:
    def __init__(self, history_file: str):
        self.history_file = history_file
        
    def load_history(self) -> Dict:
        """Load deployment history from file"""
        if not os.path.exists(self.history_file):
            return {}
            
        with open(self.history_file, 'r') as f:
            return json.load(f)
    
    def format_timestamp(self, timestamp: str) -> str:
        """Format ISO timestamp to human readable"""
        try:
            dt = datetime.fromisoformat(timestamp)
            return dt.strftime("%Y-%m-%d %H:%M:%S")
        except:
            return timestamp
    
    def show_all_history(self):
        """Display all deployment history"""
        history = self.load_history()
        
        if not history:
            print("📭 No deployment history found")
            return
            
        print("\n🗂️  D9 Contract Deployment History")
        print("=" * 80)
        
        for contract, deployments in history.items():
            print(f"\n📦 {contract}")
            print("-" * 40)
            
            for idx, deployment in enumerate(reversed(deployments)):
                print(f"\n  [{len(deployments) - idx}] {self.format_timestamp(deployment['timestamp'])}")
                print(f"      Network: {deployment['network']}")
                print(f"      Branch: {deployment['branch']}")
                print(f"      Code Hash: {deployment['code_hash']}")
                
                if deployment.get('contract_address'):
                    print(f"      Address: {deployment['contract_address']}")
                else:
                    print(f"      Type: Code Upload Only")
                    
                print(f"      Deployed By: {deployment.get('deployed_by', 'unknown')}")
                print(f"      Git Commit: {deployment['git_commit'][:8]}")
                
                if deployment.get('previous_code_hash'):
                    print(f"      Previous Hash: {deployment['previous_code_hash']}")
    
    def show_contract_history(self, contract: str):
        """Display history for specific contract"""
        history = self.load_history()
        
        if contract not in history:
            print(f"❌ No deployment history found for {contract}")
            return
            
        deployments = history[contract]
        
        print(f"\n📦 Deployment History for {contract}")
        print("=" * 80)
        
        for idx, deployment in enumerate(reversed(deployments)):
            print(f"\n[{len(deployments) - idx}] {self.format_timestamp(deployment['timestamp'])}")
            print(f"    Network: {deployment['network']}")
            print(f"    Branch: {deployment['branch']}")
            print(f"    Code Hash: {deployment['code_hash']}")
            
            if deployment.get('contract_address'):
                print(f"    Address: {deployment['contract_address']}")
            else:
                print(f"    Type: Code Upload Only")
                
            print(f"    Deployed By: {deployment.get('deployed_by', 'unknown')}")
            print(f"    Git Commit: {deployment['git_commit'][:8]}")
            
            if deployment.get('previous_code_hash'):
                print(f"    Previous Hash: {deployment['previous_code_hash']}")
    
    def show_latest_deployments(self):
        """Show only the latest deployment for each contract"""
        history = self.load_history()
        
        if not history:
            print("📭 No deployment history found")
            return
            
        print("\n📌 Latest Deployments")
        print("=" * 80)
        
        for contract, deployments in history.items():
            if deployments:
                latest = deployments[-1]
                print(f"\n{contract}:")
                print(f"  Network: {latest['network']}")
                print(f"  Code Hash: {latest['code_hash']}")
                print(f"  Deployed: {self.format_timestamp(latest['timestamp'])}")
                
                if latest.get('contract_address'):
                    print(f"  Address: {latest['contract_address']}")
    
    def find_by_hash(self, code_hash: str):
        """Find deployments by code hash"""
        history = self.load_history()
        found = []
        
        for contract, deployments in history.items():
            for deployment in deployments:
                if deployment['code_hash'] == code_hash or deployment['code_hash'].startswith(code_hash):
                    found.append((contract, deployment))
        
        if not found:
            print(f"❌ No deployments found with hash: {code_hash}")
            return
            
        print(f"\n🔍 Deployments with hash {code_hash}:")
        print("=" * 80)
        
        for contract, deployment in found:
            print(f"\nContract: {contract}")
            print(f"  Network: {deployment['network']}")
            print(f"  Deployed: {self.format_timestamp(deployment['timestamp'])}")
            print(f"  Branch: {deployment['branch']}")


def main():
    if len(sys.argv) < 2:
        print("Usage:")
        print("  python show_history.py <history-file> [options]")
        print("\nOptions:")
        print("  --all                    Show all history (default)")
        print("  --contract <name>        Show history for specific contract")
        print("  --latest                 Show only latest deployments")
        print("  --find-hash <hash>       Find deployments by code hash")
        sys.exit(1)
    
    history_file = sys.argv[1]
    viewer = DeploymentHistoryViewer(history_file)
    
    if len(sys.argv) == 2 or "--all" in sys.argv:
        viewer.show_all_history()
    elif "--contract" in sys.argv:
        idx = sys.argv.index("--contract")
        if idx + 1 < len(sys.argv):
            viewer.show_contract_history(sys.argv[idx + 1])
        else:
            print("Error: --contract requires a contract name")
    elif "--latest" in sys.argv:
        viewer.show_latest_deployments()
    elif "--find-hash" in sys.argv:
        idx = sys.argv.index("--find-hash")
        if idx + 1 < len(sys.argv):
            viewer.find_by_hash(sys.argv[idx + 1])
        else:
            print("Error: --find-hash requires a hash value")


if __name__ == "__main__":
    main()