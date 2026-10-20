// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

// A plain ERC-20 (EIP-20), as solc 0.8 would compile it: the same source
// runs natively here and as EVM bytecode in the revm benchmark.
contract erc20 {
	string public constant name = "Prova Token";
	string public constant symbol = "PRV";
	uint8 public constant decimals = 18;

	uint256 public totalSupply;
	mapping(address => uint256) public balanceOf;
	mapping(address => mapping(address => uint256)) public allowance;

	event Transfer(address indexed from, address indexed to, uint256 value);
	event Approval(address indexed owner, address indexed spender, uint256 value);

	constructor(uint256 initialSupply) {
		totalSupply = initialSupply;
		balanceOf[msg.sender] = initialSupply;
		emit Transfer(address(0), msg.sender, initialSupply);
	}

	function transfer(address to, uint256 value) public returns (bool) {
		_transfer(msg.sender, to, value);
		return true;
	}

	function approve(address spender, uint256 value) public returns (bool) {
		require(spender != address(0), "ERC20: approve to the zero address");
		allowance[msg.sender][spender] = value;
		emit Approval(msg.sender, spender, value);
		return true;
	}

	function transferFrom(address from, address to, uint256 value) public returns (bool) {
		uint256 allowed = allowance[from][msg.sender];
		if (allowed != type(uint256).max) {
			require(allowed >= value, "ERC20: insufficient allowance");
			allowance[from][msg.sender] = allowed - value;
		}
		_transfer(from, to, value);
		return true;
	}

	function _transfer(address from, address to, uint256 value) internal {
		require(to != address(0), "ERC20: transfer to the zero address");
		uint256 balance = balanceOf[from];
		require(balance >= value, "ERC20: transfer amount exceeds balance");
		balanceOf[from] = balance - value;
		balanceOf[to] += value;
		emit Transfer(from, to, value);
	}
}
