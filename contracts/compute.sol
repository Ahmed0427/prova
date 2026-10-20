// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

// Compute-heavy functions for the benchmark: no storage, so the cost is the
// arithmetic, loops and memory of each execution model.
contract compute {
	// uint256 additions.
	function fib(uint256 n) public pure returns (uint256 a) {
		uint256 b = 1;
		for (uint256 i = 0; i < n; i++) {
			(a, b) = (b, a + b);
		}
	}

	// uint256 multiplication, division and modulo.
	function primes(uint256 n) public pure returns (uint256 count) {
		for (uint256 i = 2; i < n; i++) {
			bool prime = true;
			for (uint256 j = 2; j * j <= i; j++) {
				if (i % j == 0) {
					prime = false;
					break;
				}
			}
			if (prime) {
				count++;
			}
		}
	}

	// Memory: a bubble sort of n pseudo-random words.
	function sort(uint256 n) public pure returns (uint256 checksum) {
		uint256[] memory a = new uint256[](n);
		uint256 x = 12345;
		for (uint256 i = 0; i < n; i++) {
			x = (x * 1103515245 + 12345) % 2147483648;
			a[i] = x;
		}
		for (uint256 i = 0; i < n; i++) {
			for (uint256 j = 0; j + 1 < n - i; j++) {
				// A swap through a temporary: Solang (every target) fails on
				// the tuple form (a[j], a[j + 1]) = (a[j + 1], a[j]).
				if (a[j] > a[j + 1]) {
					uint256 t = a[j];
					a[j] = a[j + 1];
					a[j + 1] = t;
				}
			}
		}
		for (uint256 i = 0; i < n; i++) {
			unchecked {
				checksum = checksum * 31 + a[i];
			}
		}
	}

	// keccak256 in a loop: the precompile in both models.
	function hashChain(uint256 n) public pure returns (bytes32 h) {
		for (uint256 i = 0; i < n; i++) {
			h = keccak256(abi.encode(h, i));
		}
	}
}
