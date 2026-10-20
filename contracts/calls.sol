// Calls between native contracts. A revert in a callee undoes only the
// callee's writes and events; the caller can catch it and go on.
contract callee {
	uint256 public value;

	event Set(uint256 value);

	function set(uint256 v) public returns (uint256) {
		value = v;
		emit Set(v);
		return v * 2;
	}

	// Writes, emits, then reverts unless v is small: the write can't be
	// optimized away, so the caller sees it undone.
	function setAndRevert(uint256 v) public {
		value = v;
		emit Set(v);
		if (v >= 5) {
			revert("callee reverted");
		}
	}

	function whoCalls() public view returns (address, address) {
		return (msg.sender, address(this));
	}

	// Calls back into the contract that called it.
	function callBack(caller c) public {
		value += 1;
		c.bump();
	}
}

contract caller {
	callee public target;
	uint256 public steps;

	event Caught(string reason);

	constructor(callee t) {
		target = t;
	}

	function callSet(uint256 v) public returns (uint256) {
		steps += 1;
		return target.set(v);
	}

	// B reverts, A catches it and continues: B's write and event are gone.
	function tryRevert(uint256 v) public returns (bool ok, uint256 seen) {
		steps += 1;
		try target.setAndRevert(v) {
			ok = true;
		} catch Error(string memory reason) {
			emit Caught(reason);
			ok = false;
		}
		seen = target.value();
	}

	function rawRevert(uint256 v) public returns (bool ok, bytes memory data) {
		steps += 1;
		(ok, data) = address(target).call(abi.encodeWithSignature("setAndRevert(uint256)", v));
	}

	// Not caught: the revert, with the callee's data, ends the transaction.
	function uncaught(uint256 v) public {
		steps += 1;
		target.setAndRevert(v);
	}

	function who() public view returns (address, address) {
		return target.whoCalls();
	}

	function reenter() public {
		steps += 1;
		target.callBack(this);
		steps += 10;
	}

	function bump() public {
		steps += 100;
	}

	// A static call may read, but a write anywhere below it fails the call.
	function staticRead() public returns (bool ok, bytes memory data) {
		(ok, data) = address(target).staticcall(abi.encodeWithSignature("value()"));
	}

	function staticWrite(uint256 v) public returns (bool ok) {
		(ok, ) = address(target).staticcall(abi.encodeWithSignature("set(uint256)", v));
	}

	// The callee's code on this contract's storage: set() writes slot 0 here,
	// which is `target`.
	function delegateSet(uint256 v) public returns (bool ok) {
		(ok, ) = address(target).delegatecall(abi.encodeWithSignature("set(uint256)", v));
	}
}
