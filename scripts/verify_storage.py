#!/usr/bin/env python3
"""
Storage Layout Verification for ink! Contracts
Ensures storage fields maintain order and no additions/removals
"""

import re
import json
import sys
import os
from typing import List, Tuple, Optional
from dataclasses import dataclass

@dataclass
class StorageField:
    name: str
    field_type: str
    position: int
    
    def __eq__(self, other):
        return (self.name == other.name and 
                self.field_type == other.field_type and
                self.position == other.position)

class StorageLayoutChecker:
    def __init__(self):
        self.layout_file = "storage-layouts.json"
        
    def extract_storage_fields(self, contract_path: str) -> List[StorageField]:
        """Extract ordered list of storage fields from contract"""
        lib_path = os.path.join(contract_path, "lib.rs")
        
        with open(lib_path, 'r') as f:
            content = f.read()
        
        # Find the storage struct
        storage_pattern = r'#\[ink\(storage\)\]\s*pub struct \w+ \{(.*?)\}'
        match = re.search(storage_pattern, content, re.DOTALL)
        
        if not match:
            raise ValueError(f"No storage struct found in {contract_path}")
        
        storage_content = match.group(1)
        
        # Extract fields with their types
        # Handle different patterns:
        # - Simple: `admin: AccountId,`
        # - With visibility: `pub admin: AccountId,`
        # - With attributes: `#[storage_field] admin: AccountId,`
        # - Complex types: `balances: Mapping<AccountId, Balance>,`
        
        field_pattern = r'(?:pub\s+)?(\w+)\s*:\s*([^,]+),'
        
        fields = []
        position = 0
        
        # Remove comments first
        storage_content = re.sub(r'//.*', '', storage_content, flags=re.MULTILINE)
        storage_content = re.sub(r'/\*.*?\*/', '', storage_content, flags=re.DOTALL)
        
        for match in re.finditer(field_pattern, storage_content):
            field_name = match.group(1).strip()
            field_type = match.group(2).strip()
            
            # Skip if it's a comment or attribute
            if field_name.startswith('//') or field_name.startswith('#'):
                continue
                
            fields.append(StorageField(
                name=field_name,
                field_type=field_type,
                position=position
            ))
            position += 1
        
        return fields
    
    def load_saved_layouts(self) -> dict:
        """Load previously saved storage layouts"""
        if os.path.exists(self.layout_file):
            with open(self.layout_file, 'r') as f:
                data = json.load(f)
                # Convert back to StorageField objects
                for contract, fields in data.items():
                    data[contract] = [
                        StorageField(**field) for field in fields
                    ]
                return data
        return {}
    
    def save_layout(self, contract_name: str, fields: List[StorageField]):
        """Save storage layout for future comparison"""
        layouts = self.load_saved_layouts()
        
        # Convert to dict for JSON serialization
        layouts[contract_name] = [
            {
                "name": f.name,
                "field_type": f.field_type,
                "position": f.position
            }
            for f in fields
        ]
        
        with open(self.layout_file, 'w') as f:
            json.dump(layouts, f, indent=2)
    
    def verify_storage_unchanged(self, contract_name: str) -> Tuple[bool, Optional[str]]:
        """Verify storage layout hasn't changed"""
        try:
            current_fields = self.extract_storage_fields(contract_name)
        except Exception as e:
            return False, f"Failed to extract storage: {e}"
        
        saved_layouts = self.load_saved_layouts()
        
        # If no saved layout, this is the first time
        if contract_name not in saved_layouts:
            print(f"📝 First time checking {contract_name}, saving layout...")
            self.save_layout(contract_name, current_fields)
            return True, None
        
        saved_fields = saved_layouts[contract_name]
        
        # Check for changes
        errors = []
        
        # Check length
        if len(current_fields) != len(saved_fields):
            errors.append(
                f"Number of fields changed: {len(saved_fields)} → {len(current_fields)}"
            )
            
            # Find added/removed fields
            saved_names = {f.name for f in saved_fields}
            current_names = {f.name for f in current_fields}
            
            added = current_names - saved_names
            removed = saved_names - current_names
            
            if added:
                errors.append(f"Added fields: {', '.join(added)}")
            if removed:
                errors.append(f"Removed fields: {', '.join(removed)}")
        
        # Check order and types
        for i, (saved, current) in enumerate(zip(saved_fields, current_fields)):
            if saved.position != current.position:
                errors.append(
                    f"Field '{saved.name}' moved from position {saved.position} to {current.position}"
                )
            
            if saved.name != current.name:
                errors.append(
                    f"Field at position {i} changed: '{saved.name}' → '{current.name}'"
                )
            
            if saved.field_type != current.field_type:
                errors.append(
                    f"Field '{saved.name}' type changed: '{saved.field_type}' → '{current.field_type}'"
                )
        
        if errors:
            return False, "\n".join(errors)
        
        return True, None
    
    
    def show_layout(self, contract_name: str):
        """Display the storage layout"""
        try:
            fields = self.extract_storage_fields(contract_name)
            print(f"\n📋 Storage layout for {contract_name}:")
            print("-" * 50)
            for field in fields:
                print(f"  [{field.position}] {field.name}: {field.field_type}")
        except Exception as e:
            print(f"❌ Error: {e}")


def main():
    if len(sys.argv) < 3:
        print("Usage:")
        print("  python verify_storage.py check <contract>")
        print("  python verify_storage.py show <contract>")
        sys.exit(1)
    
    command = sys.argv[1]
    contract = sys.argv[2]
    
    checker = StorageLayoutChecker()
    
    if command == "check":
        ok, error = checker.verify_storage_unchanged(contract)
        if ok:
            print(f"✅ Storage layout unchanged for {contract}")
        else:
            print(f"❌ Storage layout changed for {contract}:")
            print(error)
            sys.exit(1)
    
    elif command == "show":
        checker.show_layout(contract)
    
    else:
        print(f"Unknown command: {command}")
        sys.exit(1)


if __name__ == "__main__":
    main()